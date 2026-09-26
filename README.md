# sqlx-cloudflare

> sqlx drivers for Cloudflare Workers: real `sqlx::Database` implementations,
> so code written against sqlx's runtime API runs inside a Rust Worker.

[![CI](https://github.com/sqlx-contrib/sqlx-cloudflare/actions/workflows/ci.yml/badge.svg)](https://github.com/sqlx-contrib/sqlx-cloudflare/actions/workflows/ci.yml)
[![Rust (edition 2021)](https://img.shields.io/badge/Rust-2021-black?logo=rust)](https://www.rust-lang.org/)
[![Nix Flake](https://img.shields.io/badge/Nix-Flake-5277C3?logo=nixos&logoColor=white)](https://nixos.wiki/wiki/Flakes)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

> [!NOTE]
> **Pre-1.0.** The API may still change between minor versions.

| Crate | Version | Docs | What it is |
|---|---|---|---|
| [`sqlx-cloudflare-d1`](crates/sqlx-cloudflare-d1) | [![crates.io](https://img.shields.io/crates/v/sqlx-cloudflare-d1.svg)](https://crates.io/crates/sqlx-cloudflare-d1) | [![docs.rs](https://img.shields.io/docsrs/sqlx-cloudflare-d1)](https://docs.rs/sqlx-cloudflare-d1) | A driver for Cloudflare D1, over the Workers D1 binding |
| [`sqlx-cloudflare-do`](crates/sqlx-cloudflare-do) | [![crates.io](https://img.shields.io/crates/v/sqlx-cloudflare-do.svg)](https://crates.io/crates/sqlx-cloudflare-do) | [![docs.rs](https://img.shields.io/docsrs/sqlx-cloudflare-do)](https://docs.rs/sqlx-cloudflare-do) | A driver for a Durable Object's SQL storage |
| [`sqlx-cloudflare-core`](crates/sqlx-cloudflare-core) | [![crates.io](https://img.shields.io/crates/v/sqlx-cloudflare-core.svg)](https://crates.io/crates/sqlx-cloudflare-core) | [![docs.rs](https://img.shields.io/docsrs/sqlx-cloudflare-core)](https://docs.rs/sqlx-cloudflare-core) | The internals both drivers share -- not a dependency of yours |

Each driver is independent: depend on the one for the storage you use. Both
are SQLite behind a JavaScript API, so they share their value model, type
mapping and error parsing through `sqlx-cloudflare-core`.

## Which one?

- **D1** is one database your Worker talks to over the network: shared by
  every request, queryable across all your data, managed with `wrangler d1`.
  Atomic work goes through a batch -- D1 has no interactive transactions.
- **Durable Object storage** is a private SQLite database inside each object,
  in the same process as your code: queries take microseconds, every request
  to an object sees one consistent database, and transactions are real. Pick
  it for per-user, per-tenant or per-room data.

Both drivers map types the same way and share the same limit on integers
(±2^53 − 1, since values cross as JavaScript numbers).

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
