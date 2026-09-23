//! `Encode`, `Decode` and `Type` for the Rust types D1 can store.
//!
//! D1 is SQLite behind a JavaScript API, so the mapping follows sqlx-sqlite
//! wherever D1 does not force a difference -- code moving from `Sqlite` to
//! `D1` should behave the same. Where it does force one, it fails loudly:
//!
//! | Rust | Stored as | Notes |
//! |---|---|---|
//! | `bool` | `INTEGER` 0/1 | any non-zero integer decodes as `true` |
//! | `i8`, `i16`, `i32`, `u8`, `u16`, `u32` | `INTEGER` | decoding a value that does not fit is an error |
//! | `i64` | `INTEGER` | only within ±(2^53 − 1), in both directions |
//! | `f32`, `f64` | `REAL` | also decode from `INTEGER` |
//! | `&str`, `String`, `Box<str>`, `Arc<str>`, `Cow<str>` | `TEXT` | |
//! | `&[u8]`, `Vec<u8>`, `Box<[u8]>`, `Arc<[u8]>` | `BLOB` | also decode from `TEXT`, as its bytes |
//!
//! `u64` is deliberately missing: there is nothing it could be stored as
//! without losing most of its range.
//!
//! # Integers and JavaScript numbers
//!
//! D1 passes values as JavaScript numbers, never `BigInt`, and a number is
//! exact only up to 2^53 − 1. So binding an `i64` outside ±(2^53 − 1) is an
//! error, not a value rounded on the way in, and a stored integer beyond that
//! range comes back as a `REAL` that will not decode as `i64`.
//!
//! For the same reason, a whole number stored as `REAL` -- `3.0` -- arrives
//! indistinguishable from the integer `3`, and is typed `INTEGER`. That is why
//! the float types decode from `INTEGER` too.

mod bool;
mod bytes;
mod float;
mod int;
mod str;
