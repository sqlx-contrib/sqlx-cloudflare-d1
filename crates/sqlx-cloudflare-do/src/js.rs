//! Everything that touches a JavaScript value.
//!
//! The rest of the crate is plain Rust over `DoArgumentValue` and `DoValue`,
//! which is what lets it be unit-tested on the host. This module is the one
//! conversion point in each direction -- through `sqlx_cloudflare_core::js`,
//! which D1 shares -- and the only place a `!Send` handle exists.
//!
//! Unlike D1's, nothing here is asynchronous: `sql.exec` runs the statement
//! before it returns, and the cursor it hands back is read to the end on the
//! spot. That is also what makes it safe -- a cursor held across an `await`
//! does not see a stable snapshot.
//!
//! It goes to `worker-sys` directly rather than through `worker`'s
//! `SqlStorage::exec`, which only takes its own value type and would convert
//! every argument a second time.

use js_sys::Array;
use sqlx_cloudflare_core::js::{database_error, from_js, to_js_array};
use sqlx_cloudflare_core::{QueryResult, Value};
use sqlx_core::error::Error;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use worker::worker_sys::types::{SqlStorage as SqlStorageSys, SqlStorageCursor};

use crate::{DoArgumentValue, DoRow};

/// Runs `sql` with `arguments` bound in placeholder order, and returns its
/// rows -- at most `limit` of them -- and the rows it wrote.
///
/// Every row is read out of the cursor whatever `limit` says: a statement
/// like `INSERT ... RETURNING` only finishes as its cursor is drained, so
/// stopping early could leave it half done. `limit` only saves converting
/// rows nobody will read.
pub(crate) fn fetch(
    storage: &worker::SqlStorage,
    sql: &str,
    arguments: &[DoArgumentValue],
    limit: Option<usize>,
) -> Result<(Vec<DoRow>, u64), Error> {
    let cursor = exec(storage, sql, arguments)?;

    let names = cursor
        .column_names()
        .iter()
        .map(|name| {
            name.as_string().ok_or_else(|| {
                Error::Protocol(
                    "Durable Object storage returned a column name that is not a string".into(),
                )
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let limit = limit.unwrap_or(usize::MAX);
    let mut rows = Vec::new();
    let raw = cursor.raw();

    loop {
        let next = raw.next().map_err(|error| database_error(&error))?;
        if next.done() {
            break;
        }
        if rows.len() < limit {
            let row: Array = next.value().dyn_into().map_err(|_| {
                Error::Protocol("Durable Object storage returned a row that is not an array".into())
            })?;
            rows.push(
                row.iter()
                    .map(|value| from_js(&value))
                    .collect::<Result<Vec<Value>, _>>()?,
            );
        }
    }

    Ok((
        DoRow::from_result(names, rows)?,
        whole_number_u64(cursor.rows_written()),
    ))
}

/// Runs `sql` for its effect, and returns what it did.
///
/// The cursor reports rows written but neither count SQLite keeps, so those
/// come from `changes()` and `last_insert_rowid()` right after -- still
/// synchronously, so nothing can run in between. Only after a statement that
/// wrote: `changes()` keeps the count of the last write, and a `SELECT` must
/// not report the `INSERT` before it.
pub(crate) fn run(
    storage: &worker::SqlStorage,
    sql: &str,
    arguments: &[DoArgumentValue],
) -> Result<QueryResult, Error> {
    let (_, rows_written) = fetch(storage, sql, arguments, Some(0))?;

    if rows_written == 0 {
        return Ok(QueryResult::default());
    }

    let (rows, _) = fetch(storage, "SELECT changes(), last_insert_rowid()", &[], None)?;
    let row = rows.first().ok_or_else(|| {
        Error::Protocol("`SELECT changes(), last_insert_rowid()` returned no row".into())
    })?;

    let changes = row.values[0].int().map_err(Error::Decode)?;
    let rowid = row.values[1].int().map_err(Error::Decode)?;

    Ok(QueryResult::new(
        u64::try_from(changes).map_err(|error| Error::Decode(error.into()))?,
        Some(rowid),
    ))
}

fn exec(
    storage: &worker::SqlStorage,
    sql: &str,
    arguments: &[DoArgumentValue],
) -> Result<SqlStorageCursor, Error> {
    let storage: &SqlStorageSys = AsRef::<JsValue>::as_ref(storage).unchecked_ref();
    let arguments = to_js_array(arguments)?;

    storage
        .exec(sql, arguments)
        .map_err(|error| database_error(&error))
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a count of rows written is whole, non-negative and far below 2^53"
)]
fn whole_number_u64(value: f64) -> u64 {
    value as u64
}
