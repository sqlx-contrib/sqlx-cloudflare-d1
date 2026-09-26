# sqlx-cloudflare

> sqlx drivers for Cloudflare Workers: real `sqlx::Database` implementations,
> so code written against sqlx's runtime API runs inside a Rust Worker.

[![CI](https://github.com/sqlx-contrib/sqlx-cloudflare/actions/workflows/ci.yml/badge.svg)](https://github.com/sqlx-contrib/sqlx-cloudflare/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

> [!NOTE]
> **Pre-release.** Not on crates.io yet, and the API may still move.

| Crate | What it drives |
|---|---|
| [`sqlx-cloudflare-d1`](crates/sqlx-cloudflare-d1) | Cloudflare D1, over the Workers D1 binding |

Each crate is independent: depend on the one for the storage you use.

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
