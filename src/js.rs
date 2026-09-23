//! Everything that touches a JavaScript value.
//!
//! The rest of the crate is plain Rust over `D1ArgumentValue` and `D1Value`,
//! which is what lets it be unit-tested on the host. This module is the one
//! conversion point in each direction, and the only place a `!Send` handle or
//! future exists -- the executor wraps every future that comes out of here in
//! `worker::send::SendFuture`, which is sound only because a Worker runs on
//! one thread.
//!
//! It goes to `worker-sys` directly rather than through `worker`'s wrappers:
//! `D1Database::prepare` there unwraps the JavaScript result, so a statement D1
//! rejected up front would abort the Worker instead of returning an error.

use js_sys::futures::JsFuture;
use js_sys::{Array, ArrayBuffer, Object, Promise, Reflect, Uint8Array};
use sqlx_core::error::Error;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use worker::worker_sys::types::{
    D1Database as D1DatabaseSys, D1PreparedStatement as D1PreparedStatementSys,
    D1Result as D1ResultSys,
};

use crate::error::D1DatabaseError;
use crate::value::{safe_integer, D1ValueData};
use crate::{D1ArgumentValue, D1QueryResult, D1Row, D1Value};

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

    let values = arguments
        .iter()
        .map(to_js)
        .collect::<Result<Array, Error>>()?;

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
/// single transaction -- if one fails, none of them take effect.
pub(crate) async fn batch(
    db: &worker::D1Database,
    statements: Vec<D1PreparedStatementSys>,
) -> Result<Vec<D1QueryResult>, Error> {
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
        .map(|result| query_result(result.unchecked_ref()))
        .collect()
}

fn query_result(result: &D1ResultSys) -> Result<D1QueryResult, Error> {
    let meta = result.meta().map_err(|error| protocol_error(&error))?;
    let number = |key: &str| {
        Reflect::get(&meta, &key.into())
            .ok()
            .and_then(|value| value.as_f64())
    };

    Ok(D1QueryResult {
        rows_affected: number("changes").map_or(0, whole_number_u64),
        last_insert_rowid: number("last_row_id").map(whole_number_i64),
    })
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

fn to_js(value: &D1ArgumentValue) -> Result<JsValue, Error> {
    Ok(match value {
        D1ArgumentValue::Null => JsValue::NULL,
        // Checked again here, not only in `Encode for i64`: a third-party
        // `Encode` impl can push any `Integer` it likes.
        D1ArgumentValue::Integer(value) => {
            #[allow(
                clippy::cast_precision_loss,
                reason = "`safe_integer` has just checked the conversion is exact"
            )]
            let value = safe_integer(*value).map_err(Error::Encode)? as f64;
            JsValue::from_f64(value)
        }
        D1ArgumentValue::Real(value) => JsValue::from_f64(*value),
        D1ArgumentValue::Text(value) => JsValue::from_str(value),
        // An `ArrayBuffer`, which D1 documents as the type it stores as BLOB.
        // A fresh `Uint8Array` owns a buffer of exactly its length.
        D1ArgumentValue::Blob(value) => Uint8Array::from(value.as_slice()).buffer().into(),
    })
}

fn from_js(value: &JsValue) -> Result<D1Value, Error> {
    let data = if value.is_null() || value.is_undefined() {
        D1ValueData::Null
    } else if let Some(number) = value.as_f64() {
        D1ValueData::from_number(number)
    } else if let Some(text) = value.as_string() {
        D1ValueData::Text(text)
    } else if let Some(boolean) = value.as_bool() {
        D1ValueData::Integer(i64::from(boolean))
    } else if let Some(buffer) = value.dyn_ref::<ArrayBuffer>() {
        D1ValueData::Blob(Uint8Array::new(buffer).to_vec())
    } else if let Some(bytes) = value.dyn_ref::<Uint8Array>() {
        D1ValueData::Blob(bytes.to_vec())
    } else if let Some(numbers) = value.dyn_ref::<Array>() {
        // D1 has returned BLOBs as plain arrays of byte values.
        D1ValueData::Blob(bytes_from_numbers(numbers)?)
    } else {
        return Err(Error::Decode(
            format!("D1 returned a value of an unexpected JavaScript type: {value:?}").into(),
        ));
    };

    Ok(D1Value(data))
}

fn bytes_from_numbers(numbers: &Array) -> Result<Vec<u8>, Error> {
    numbers
        .iter()
        .map(|number| {
            number
                .as_f64()
                .filter(|number| number.fract() == 0.0 && (0.0..=255.0).contains(number))
                .map(|number| {
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "the filter has just checked it is a whole number in 0..=255"
                    )]
                    let byte = number as u8;
                    byte
                })
                .ok_or_else(|| {
                    Error::Decode("D1 returned a BLOB element that is not a byte".into())
                })
        })
        .collect()
}

fn array(value: JsValue) -> Result<Array, Error> {
    value
        .dyn_into()
        .map_err(|_| Error::Protocol("D1 returned a row that is not an array".into()))
}

/// An error thrown by a D1 call.
///
/// D1 wraps SQLite's message: the thrown error's `message` is D1's summary,
/// and its `cause`, when there is one, carries SQLite's own text -- the part
/// the `ErrorKind` mapping reads -- so both are kept.
fn database_error(error: &JsValue) -> Error {
    let message = match error.dyn_ref::<js_sys::Error>() {
        Some(error) => {
            let message = String::from(error.message());

            match error.cause().dyn_ref::<js_sys::Error>() {
                Some(cause) => {
                    let cause = String::from(cause.message());
                    if message.contains(&cause) {
                        message
                    } else {
                        format!("{message}: {cause}")
                    }
                }
                None => message,
            }
        }
        None => error.as_string().unwrap_or_else(|| format!("{error:?}")),
    };

    Error::Database(Box::new(D1DatabaseError::new(message)))
}

fn protocol_error(error: &JsValue) -> Error {
    Error::Protocol(format!("unexpected response from D1: {error:?}"))
}
