# sqlx-cloudflare-core

The shared internals of the sqlx-cloudflare drivers. You want one of those
instead:

- [`sqlx-cloudflare-d1`](https://crates.io/crates/sqlx-cloudflare-d1), for
  Cloudflare D1;
- [`sqlx-cloudflare-do`](https://crates.io/crates/sqlx-cloudflare-do), for
  Durable Object SQL storage.

Both are SQLite behind a JavaScript API, so they share the value model, the
type mapping, the conversion to and from JavaScript and the error parsing --
which is what lives here. This crate's API serves those drivers and carries no
stability promise of its own.

## License

[MIT](https://github.com/sqlx-contrib/sqlx-cloudflare/blob/main/LICENSE)
