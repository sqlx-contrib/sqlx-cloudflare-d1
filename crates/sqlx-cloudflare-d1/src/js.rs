//! Everything that touches a JavaScript value.
//!
//! The rest of the crate is plain Rust over `D1ArgumentValue` and `D1Value`,
//! which is what lets it be unit-tested on the host. This module is the one
//! conversion point in each direction -- through `sqlx_cloudflare_core::js`,
//! which Durable Object storage shares -- and the only place a `!Send` handle or
//! future exists -- the executor wraps every future that comes out of here in
//! `worker::send::SendFuture`, which is sound only because a Worker runs on
//! one thread.
//!
//! It goes to `worker-sys` directly rather than through `worker`'s wrappers:
//! `D1Database::prepare` there unwraps the JavaScript result, so a statement D1
//! rejected up front would abort the Worker instead of returning an error.

use js_sys::futures::JsFuture;
use js_sys::{Array, Object, Promise, Reflect};
use sqlx_cloudflare_core::js::{database_error, from_js, to_js_array};
use sqlx_cloudflare_core::{BatchResult, QueryResult};
use sqlx_core::error::Error;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use worker::worker_sys::types::{
    D1Database as D1DatabaseSys, D1PreparedStatement as D1PreparedStatementSys,
    D1Result as D1ResultSys,
};

use crate::{D1ArgumentValue, D1BatchResult, D1QueryResult, D1Row};

#[wasm_bindgen]
extern "C" {
    /// Our own view of a `D1PreparedStatement`, to hang a binding on.
    ///
    /// A `method` on `worker-sys`'s type would be an inherent impl on a
    /// foreign type, which the orphan rule forbids -- so this declares a local
    /// type for the same JavaScript object and statements are cast to it.
    /// Methods are looked up by name at call time, so the cast costs nothing.
    #[wasm_bindgen(extends = Object)]
    type Statement;

    // `raw({ columnNames: true })`, which returns the column names as the
    // first row. `worker-sys` binds `raw()` with no arguments, and without the
    // option the names are lost -- while `all()`, which keeps them, returns
    // each row as an object and so collapses `SELECT a.id, b.id` to one `id`.
    #[wasm_bindgen(method, catch, js_name = raw)]
    fn raw_with_options(this: &Statement, options: &JsValue) -> Result<Promise, JsValue>;
}

/// `sql` prepared on `db`, with `arguments` bound in placeholder order.
pub(crate) fn prepare(
    db: &worker::D1Database,
    sql: &str,
    arguments: &[D1ArgumentValue],
) -> Result<D1PreparedStatementSys, Error> {
    let db: &D1DatabaseSys = AsRef::<JsValue>::as_ref(db).unchecked_ref();
    let statement = db.prepare(sql).map_err(|error| database_error(&error))?;

    if arguments.is_empty() {
        return Ok(statement);
    }

    let values = to_js_array(arguments)?;

    statement
        .bind(values)
        .map_err(|error| database_error(&error))
}

/// Runs `statement` and returns its rows, at most `limit` of them.
///
/// D1 returns the whole result at once -- there is no cursor -- so `limit`
/// only saves converting rows nobody will read, not fetching them.
pub(crate) async fn fetch(
    statement: &D1PreparedStatementSys,
    limit: Option<usize>,
) -> Result<Vec<D1Row>, Error> {
    let options = Object::new();
    Reflect::set(&options, &"columnNames".into(), &JsValue::TRUE)
        .map_err(|error| protocol_error(&error))?;

    let promise = statement
        .unchecked_ref::<Statement>()
        .raw_with_options(&options)
        .map_err(|error| database_error(&error))?;
    let result: Array = JsFuture::from(promise)
        .await
        .map_err(|error| database_error(&error))?
        .dyn_into()
        .map_err(|_| Error::Protocol("D1 `raw()` did not return an array".into()))?;

    let mut result = result.iter();

    // No first row means no column names either; there is nothing to return.
    let Some(names) = result.next() else {
        return Ok(Vec::new());
    };

    let names = array(names)?
        .iter()
        .map(|name| {
            name.as_string().ok_or_else(|| {
                Error::Protocol("D1 returned a column name that is not a string".into())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let rows = result
        .take(limit.unwrap_or(usize::MAX))
        .map(|row| array(row)?.iter().map(|value| from_js(&value)).collect())
        .collect::<Result<Vec<Vec<_>>, _>>()?;

    D1Row::from_result(names, rows)
}

/// Runs `statement` for its effect, and returns what D1 says it did.
pub(crate) async fn run(statement: &D1PreparedStatementSys) -> Result<D1QueryResult, Error> {
    let promise = statement.run().map_err(|error| database_error(&error))?;
    let result: D1ResultSys = JsFuture::from(promise)
        .await
        .map_err(|error| database_error(&error))?
        .unchecked_into();

    query_result(&result)
}

/// Runs `statements` as one D1 batch: in order, in one round trip, and as a
/// single transaction -- if one fails, none of them take effect. Returns what
/// each statement did and the rows it returned.
pub(crate) async fn batch(
    db: &worker::D1Database,
    statements: Vec<D1PreparedStatementSys>,
) -> Result<Vec<D1BatchResult>, Error> {
    let db: &D1DatabaseSys = AsRef::<JsValue>::as_ref(db).unchecked_ref();
    let statements = statements.into_iter().collect::<Array>();

    let promise = db
        .batch(statements)
        .map_err(|error| database_error(&error))?;
    let results = array(
        JsFuture::from(promise)
            .await
            .map_err(|error| database_error(&error))?,
    )?;

    results
        .iter()
        .map(|result| {
            let result: &D1ResultSys = result.unchecked_ref();
            Ok(BatchResult::new(rows(result)?, query_result(result)?))
        })
        .collect()
}

/// A batch statement's rows, which D1 returns as objects keyed by column
/// name -- `batch()` has no `raw({ columnNames: true })` to ask for arrays.
///
/// So the column names are the first row's keys, in the order JavaScript
/// keeps them, and every row is read by those names. Two columns with one
/// name are one key, and only the last survives; a key that looks like an
/// integer sorts ahead of the rest. Neither happens with arrays, which is why
/// only a batch reads rows this way.
fn rows(result: &D1ResultSys) -> Result<Vec<D1Row>, Error> {
    let Some(objects) = result.results().map_err(|error| protocol_error(&error))? else {
        return Ok(Vec::new());
    };
    let Some(first) = objects.iter().next() else {
        return Ok(Vec::new());
    };

    let names = Object::keys(first.unchecked_ref::<Object>())
        .iter()
        .map(|name| {
            name.as_string().ok_or_else(|| {
                Error::Protocol("D1 returned a column name that is not a string".into())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut rows = Vec::with_capacity(objects.length() as usize);
    for object in objects.iter() {
        let mut values = Vec::with_capacity(names.len());
        for name in &names {
            let value = Reflect::get(&object, &JsValue::from_str(name))
                .map_err(|error| protocol_error(&error))?;
            values.push(from_js(&value)?);
        }
        rows.push(values);
    }

    D1Row::from_result(names, rows)
}

fn query_result(result: &D1ResultSys) -> Result<D1QueryResult, Error> {
    let meta = result.meta().map_err(|error| protocol_error(&error))?;
    let number = |key: &str| {
        Reflect::get(&meta, &key.into())
            .ok()
            .and_then(|value| value.as_f64())
    };

    Ok(QueryResult::new(
        number("changes").map_or(0, whole_number_u64),
        number("last_row_id").map(whole_number_i64),
    ))
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "D1's counts are whole, non-negative and far below 2^53"
)]
fn whole_number_u64(value: f64) -> u64 {
    value as u64
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "a rowid D1 reports is a whole number it could represent exactly"
)]
fn whole_number_i64(value: f64) -> i64 {
    value as i64
}

fn array(value: JsValue) -> Result<Array, Error> {
    value
        .dyn_into()
        .map_err(|_| Error::Protocol("D1 returned a row that is not an array".into()))
}

fn protocol_error(error: &JsValue) -> Error {
    Error::Protocol(format!("unexpected response from D1: {error:?}"))
}
