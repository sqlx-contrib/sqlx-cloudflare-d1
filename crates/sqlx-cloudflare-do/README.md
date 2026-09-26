# sqlx-cloudflare-do

> A Cloudflare Durable Objects driver for sqlx: a real `sqlx::Database` over a
> Durable Object's SQL storage, so code written against sqlx's runtime API
> runs inside a Rust Durable Object.

[![crates.io](https://img.shields.io/crates/v/sqlx-cloudflare-do.svg)](https://crates.io/crates/sqlx-cloudflare-do)
[![docs.rs](https://img.shields.io/docsrs/sqlx-cloudflare-do)](https://docs.rs/sqlx-cloudflare-do)
[![CI](https://github.com/sqlx-contrib/sqlx-cloudflare/actions/workflows/ci.yml/badge.svg)](https://github.com/sqlx-contrib/sqlx-cloudflare/actions/workflows/ci.yml)
[![MSRV](https://img.shields.io/crates/msrv/sqlx-cloudflare-do)](https://github.com/sqlx-contrib/sqlx-cloudflare/blob/main/Cargo.toml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://github.com/sqlx-contrib/sqlx-cloudflare/blob/main/LICENSE)

> [!NOTE]
> **Pre-1.0.** The API may still change between minor versions.

```rust
use sqlx_cloudflare_do::{Do, DoConnection};
use worker::{durable_object, DurableObject, Env, Request, Response, State};

#[derive(sqlx::FromRow)]
struct User {
    id: i64,
    name: String,
}

#[durable_object]
pub struct Users {
    conn: DoConnection,
}

impl DurableObject for Users {
    fn new(state: State, _env: Env) -> Self {
        Self { conn: DoConnection::new(state.storage()) }
    }

    async fn fetch(&self, _req: Request) -> worker::Result<Response> {
        let user = sqlx::query_as::<Do, User>("SELECT id, name FROM users WHERE id = ?")
            .bind(1_i64)
            .fetch_one(&self.conn)
            .await
            .map_err(|e| worker::Error::RustError(e.to_string()))?;

        Response::ok(user.name)
    }
}
```

The class must be SQLite-backed -- `new_sqlite_classes` in `wrangler.toml`:

```toml
[[durable_objects.bindings]]
name = "USERS"
class_name = "Users"

[[migrations]]
tag = "v1"
new_sqlite_classes = ["Users"]
```

## Install

```toml
[dependencies]
sqlx-cloudflare-do = "0.2"
sqlx = { version = "0.9", default-features = false, features = ["derive"] }
worker = "0.8"
# `#[durable_object]` expands to code that names it directly.
wasm-bindgen = "0.2"
```

| sqlx-cloudflare-do | sqlx | worker | Rust |
|---|---|---|---|
| 0.2 | 0.9 | 0.8 | 1.94+ |
| 0.1 | 0.9 | 0.8 | 1.94+ |

The crate runs on `wasm32-unknown-unknown`, inside a Durable Object.

## How it differs from D1

The database lives in the Durable Object, so a query never leaves the
process: `sql.exec` runs it before returning, and the driver reads every row
out on the spot. Each query's future finishes on its first poll. Types map
exactly as in
[`sqlx-cloudflare-d1`](https://crates.io/crates/sqlx-cloudflare-d1) -- the
two share one definition.

## Transactions

`sql.exec` rejects `BEGIN`, so a transaction is a callback. It commits when
the callback returns `Ok` and rolls back every write when it returns `Err`,
and inside it you read your own writes:

```rust
let id = conn
    .transaction(move |tx| async move {
        let id: i64 = sqlx::query_scalar("INSERT INTO users (email) VALUES (?) RETURNING id")
            .bind(email)
            .fetch_one(&tx)
            .await?;
        sqlx::query("INSERT INTO posts (user_id, title) VALUES (?, 'hello')")
            .bind(id)
            .execute(&tx)
            .await?;
        Ok::<_, sqlx::Error>(id)
    })
    .await?;
```

- **The callback owns what it uses.** It is `'static` -- move data in, and run
  queries through the `tx` it is handed, not the connection.
- **It holds the whole object.** No other request reaches the object until
  the transaction ends, even while the callback awaits something that is not
  storage. That keeps other requests' writes out of it -- and is why it should
  be short: no `fetch()` inside, and never a request to the object itself.

## Batches

`execute_batch` runs statements atomically without a callback, and
`fetch_batch` streams each statement's rows and result as well -- the shape of
sqlc's `:batchexec`, `:batchone` and `:batchmany` queries:

```rust
use futures_util::TryStreamExt;
use sqlx::Row;

let names: Vec<String> = conn
    .fetch_batch([1_i64, 2, 3].map(|id| {
        sqlx::query("SELECT name FROM users WHERE id = ?").bind(id)
    }))
    .map_ok(|result| result.rows()[0].get("name"))
    .try_collect()
    .await?;
```

## Limitations

- **No `query!` / `query_as!` macros.** sqlx's macros only know its built-in
  drivers. The runtime API (`sqlx::query`, `query_as`, `FromRow`) works.
- **No `begin()`.** `sql.exec` rejects `BEGIN` and `SAVEPOINT`, so sqlx's
  `begin()` fails. Use a [transaction callback](#transactions).
- **No `sqlx::Pool`.** Nothing to pool: the connection is a handle to storage
  the object owns. Keep one in the object's struct.
- **Integers are limited to ±(2^53 − 1).** Values cross as JavaScript
  numbers, so a wider `i64` is refused rather than silently rounded. Note that
  sqlx's `QueryBuilder::push_bind` panics on a refused value instead of
  returning the error.
- **`rows_affected` comes from `changes()`.** Like SQLite's own count, it
  covers the rows the statement itself inserted, updated or deleted, not those
  its triggers touched. A statement that wrote nothing reports 0 and no
  `last_insert_rowid`.
- **No `sqlx::migrate!`.** Create the schema from the object's constructor.
- **No derived TEXT enums.** `#[derive(sqlx::Type)]` works for
  `#[sqlx(transparent)]` newtypes and `#[repr(i32)]` enums, but sqlx only
  generates string-backed enums for its built-in drivers.
- **No statement logging.** `log_statements` is accepted and ignored.

## License

[MIT](https://github.com/sqlx-contrib/sqlx-cloudflare/blob/main/LICENSE)
