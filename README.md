# sqlx-cloudflare

> sqlx drivers for Cloudflare Workers: real `sqlx::Database` implementations,
> so code written against sqlx's runtime API runs inside a Rust Worker.

[![CI](https://github.com/sqlx-contrib/sqlx-cloudflare/actions/workflows/ci.yml/badge.svg)](https://github.com/sqlx-contrib/sqlx-cloudflare/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

> [!NOTE]
> **Pre-1.0.** The API may still change between minor versions.

| Crate | What it drives |
|---|---|
| [`sqlx-cloudflare-d1`](crates/sqlx-cloudflare-d1) | Cloudflare D1, over the Workers D1 binding |
| [`sqlx-cloudflare-do`](crates/sqlx-cloudflare-do) | A Durable Object's SQL storage |

Each driver is independent: depend on the one for the storage you use. Both
are SQLite behind a JavaScript API, so they share their value model, type
mapping and error parsing through
[`sqlx-cloudflare-core`](crates/sqlx-cloudflare-core), which you do not
depend on directly.

## Development

Everything runs inside the dev shell -- `nix develop`, or the Dev Container:

```sh
make check        # host and wasm32-unknown-unknown
make lint
make test
make test-worker  # the integration tests, under `wrangler dev --local`
```

## License

[MIT](LICENSE)
