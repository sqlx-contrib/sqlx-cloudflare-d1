# sqlx-cloudflare-d1

> A Cloudflare D1 driver for sqlx: a real `sqlx::Database` over the Workers D1
> binding, so code written against sqlx's runtime API runs inside a Rust Worker.

[![CI](https://github.com/sqlx-contrib/sqlx-cloudflare/actions/workflows/ci.yml/badge.svg)](https://github.com/sqlx-contrib/sqlx-cloudflare/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://github.com/sqlx-contrib/sqlx-cloudflare/blob/main/LICENSE)

> [!NOTE]
> **Pre-release.** Not on crates.io yet, and the API may still move.

```rust
use sqlx_cloudflare_d1::{D1Connection, D1};

#[derive(sqlx::FromRow)]
struct User {
    id: i64,
    name: String,
}

#[worker::event(fetch)]
async fn fetch(_req: worker::Request, env: worker::Env, _ctx: worker::Context) -> worker::Result<worker::Response> {
    let conn = D1Connection::from_env(&env, "DB")?;

    let user = sqlx::query_as::<D1, User>("SELECT id, name FROM users WHERE id = ?")
        .bind(1_i64)
        .fetch_one(&conn)
        .await
        .map_err(|e| worker::Error::RustError(e.to_string()))?;

    worker::Response::ok(user.name)
}
```

## Install

```toml
[dependencies]
sqlx-cloudflare-d1 = "0.1"
sqlx = { version = "0.9", default-features = false, features = ["derive"] }
worker = { version = "0.8", features = ["d1"] }
```

| sqlx-cloudflare-d1 | sqlx | worker | Rust |
|---|---|---|---|
| 0.1 | 0.9 | 0.8 | 1.94+ |

The crate runs on `wasm32-unknown-unknown`, inside a Worker.

## Limitations

- **No `query!` / `query_as!` macros.** sqlx's macros only know its built-in
  drivers. The runtime API (`sqlx::query`, `query_as`, `FromRow`) works.
- **No transactions.** D1 cannot hold one open across calls, so `begin()`
  fails. Use `D1Connection::batch`, which runs statements atomically in one
  round trip.
- **No `sqlx::Pool`.** Nothing to pool: build a connection per request from
  the binding.
- **Integers are limited to ±(2^53 − 1).** D1 passes values as JavaScript
  numbers, so a wider `i64` is refused rather than silently rounded. Note that
  sqlx's `QueryBuilder::push_bind` panics on a refused value instead of
  returning the error.
- **No `sqlx::migrate!`.** Apply migrations with `wrangler d1 migrations`.
- **No derived TEXT enums.** `#[derive(sqlx::Type)]` works for
  `#[sqlx(transparent)]` newtypes and `#[repr(i32)]` enums, but sqlx only
  generates string-backed enums for its built-in drivers.
- **No statement logging.** `log_statements` is accepted and ignored.
- **Results are not streamed.** D1 returns a whole result set at once.

## License

[MIT](https://github.com/sqlx-contrib/sqlx-cloudflare/blob/main/LICENSE)
