//! A Cloudflare D1 driver for sqlx.
//!
//! [`D1`] is an [`sqlx::Database`](sqlx_core::database::Database) over the
//! Workers D1 binding, so code written against sqlx's runtime API --
//! `sqlx::query`, `sqlx::query_as`, `#[derive(sqlx::FromRow)]`, `Executor` --
//! runs inside a Rust Worker with the database type swapped.
//!
//! ```no_run
//! use sqlx_cloudflare_d1::{D1Connection, D1};
//!
//! #[derive(sqlx::FromRow)]
//! struct User {
//!     id: i64,
//!     name: String,
//! }
//!
//! async fn user(env: &worker::Env, id: i64) -> Result<User, Box<dyn std::error::Error>> {
//!     // One connection per request, straight from the binding.
//!     let conn = D1Connection::from_env(env, "DB")?;
//!
//!     let user = sqlx::query_as::<D1, User>("SELECT id, name FROM users WHERE id = ?")
//!         .bind(id)
//!         .fetch_one(&conn)
//!         .await?;
//!
//!     Ok(user)
//! }
//! ```
//!
//! # What D1 cannot do
//!
//! Each of these fails loudly rather than pretending:
//!
//! - **Transactions.** `begin()` returns an error; D1 cannot hold a
//!   transaction open across calls. [`D1Connection::batch`] runs several
//!   statements atomically in one round trip instead.
//! - **Integers beyond ±(2^53 − 1).** D1 passes values as JavaScript numbers,
//!   so binding a wider `i64` is an encode error rather than a rounded value.
//!   See [`types`].
//! - **`query!` and `describe`.** sqlx's macros only know its built-in
//!   drivers, and D1 cannot describe a statement without running it.
//!
//! There is no `sqlx::Pool` either -- a binding has nothing to pool -- and no
//! `sqlx::migrate!`: apply migrations with `wrangler d1 migrations`.
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
mod row;
mod statement;
mod transaction;
pub mod types;
mod value;

pub use arguments::D1Arguments;
pub use column::D1Column;
pub use connection::{D1ConnectOptions, D1Connection};
pub use database::D1;
pub use row::D1Row;
pub use statement::D1Statement;
pub use transaction::D1TransactionManager;
pub use value::{D1Value, D1ValueRef};

// The types D1 shares with Durable Object storage, under D1's names: both are
// SQLite behind a JavaScript API, so they are the same types underneath.
#[doc(inline)]
pub use sqlx_cloudflare_core::{
    ArgumentValue as D1ArgumentValue, DatabaseError as D1DatabaseError,
    QueryResult as D1QueryResult, TypeInfo as D1TypeInfo,
};

/// An executor for D1: `&D1Connection` or `&mut D1Connection`.
pub trait D1Executor<'c>: sqlx_core::executor::Executor<'c, Database = D1> {}
impl<'c, T: sqlx_core::executor::Executor<'c, Database = D1>> D1Executor<'c> for T {}

// NOTE: required due to the lack of lazy normalization
sqlx_core::impl_into_arguments_for_arguments!(D1Arguments);
sqlx_core::impl_column_index_for_row!(D1Row);
sqlx_core::impl_column_index_for_statement!(D1Statement);
sqlx_core::impl_acquire!(D1, D1Connection);
sqlx_core::impl_encode_for_option!(D1);
