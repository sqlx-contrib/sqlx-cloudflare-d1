# sqlx-cloudflare-do

> A Cloudflare Durable Objects driver for sqlx: a real `sqlx::Database` over a
> Durable Object's SQL storage, so code written against sqlx's runtime API
> runs inside a Rust Durable Object.

[![CI](https://github.com/sqlx-contrib/sqlx-cloudflare/actions/workflows/ci.yml/badge.svg)](https://github.com/sqlx-contrib/sqlx-cloudflare/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://github.com/sqlx-contrib/sqlx-cloudflare/blob/main/LICENSE)

> [!NOTE]
> **Pre-release.** Not on crates.io yet, and the API may still move.

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
sqlx-cloudflare-do = "0.1"
sqlx = { version = "0.9", default-features = false, features = ["derive"] }
worker = "0.8"
# `#[durable_object]` expands to code that names it directly.
wasm-bindgen = "0.2"
```

| sqlx-cloudflare-do | sqlx | worker | Rust |
|---|---|---|---|
| 0.1 | 0.9 | 0.8 | 1.94+ |

The crate runs on `wasm32-unknown-unknown`, inside a Durable Object.

## How it differs from D1

The database lives in the Durable Object, so a query never leaves the
process: `sql.exec` runs it before returning, and the driver reads every row
out on the spot. Each query's future finishes on its first poll. Types map
exactly as in [`sqlx-cloudflare-d1`](https://crates.io/crates/sqlx-cloudflare-d1) -- the two share
one definition.

## Limitations

- **No `query!` / `query_as!` macros.** sqlx's macros only know its built-in
  drivers. The runtime API (`sqlx::query`, `query_as`, `FromRow`) works.
- **No transactions.** `sql.exec` rejects `BEGIN` and `SAVEPOINT`, so
  `begin()` fails. Use `DoConnection::execute_batch`, which runs statements
  atomically inside `Storage::transaction` -- or `DoConnection::fetch_batch`,
  which streams each statement's rows and result as well, the shape of sqlc's
  `:batch*` queries.
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
