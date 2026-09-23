//! A Cloudflare D1 driver for sqlx.
//!
//! [`D1`] is an [`sqlx::Database`](sqlx_core::database::Database) over the
//! Workers D1 binding, so code written against sqlx's runtime API --
//! `sqlx::query`, `sqlx::query_as`, `#[derive(sqlx::FromRow)]`, `Executor` --
//! runs inside a Rust Worker with the database type swapped.
//!
//! This crate only *runs* on `wasm32-unknown-unknown`, inside a Worker. It
//! compiles on other targets so that docs, unit tests and `cargo check` work,
//! but its `Send` impls lean on Workers being single-threaded: do not call it
//! from a threaded host.

mod arguments;
mod batch;
mod column;
mod connection;
mod database;
mod error;
mod executor;
mod js;
mod query_result;
mod row;
mod statement;
mod transaction;
pub mod types;
mod value;

pub use arguments::{D1ArgumentValue, D1Arguments};
pub use column::D1Column;
pub use connection::{D1ConnectOptions, D1Connection};
pub use database::D1;
pub use error::D1DatabaseError;
pub use query_result::D1QueryResult;
pub use row::D1Row;
pub use statement::D1Statement;
pub use transaction::D1TransactionManager;
pub use value::{D1TypeInfo, D1Value, D1ValueRef};

/// An executor for D1: `&D1Connection` or `&mut D1Connection`.
pub trait D1Executor<'c>: sqlx_core::executor::Executor<'c, Database = D1> {}
impl<'c, T: sqlx_core::executor::Executor<'c, Database = D1>> D1Executor<'c> for T {}

// NOTE: required due to the lack of lazy normalization
sqlx_core::impl_into_arguments_for_arguments!(D1Arguments);
sqlx_core::impl_column_index_for_row!(D1Row);
sqlx_core::impl_column_index_for_statement!(D1Statement);
sqlx_core::impl_acquire!(D1, D1Connection);
sqlx_core::impl_encode_for_option!(D1);
