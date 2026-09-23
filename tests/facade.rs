//! A consumer's view: code written against the `sqlx` facade, not
//! `sqlx-core`, with `D1` as its database. It only has to compile -- there is
//! no D1 on the host to run it against -- and it proves the two crates agree
//! on one `sqlx::Database` trait.
//!
//! The dev-dependency enables sqlx's `macros` feature, sqlx's default, which
//! is the one that turns on `sqlx-core/offline` and with it the required
//! `Executor::describe`. So this also proves a consumer can keep sqlx's
//! defaults.

#![allow(dead_code)]

use sqlx_cloudflare_d1::{D1Connection, D1QueryResult, D1};

#[derive(Debug, sqlx::FromRow)]
struct User {
    id: i64,
    name: String,
    email: Option<String>,
    active: bool,
    score: f64,
    avatar: Vec<u8>,
}

// The shape `sqlc-gen-sqlx` generates: generic over the executor, naming the
// database only through the associated type.
async fn get_user<'e, E>(executor: E, id: i64) -> Result<User, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = D1>,
{
    sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ?")
        .bind(id)
        .fetch_one(executor)
        .await
}

async fn rename(conn: &mut D1Connection, id: i64, name: &str) -> Result<u64, sqlx::Error> {
    let result: D1QueryResult = sqlx::query("UPDATE users SET name = ? WHERE id = ?")
        .bind(name)
        .bind(id)
        .execute(&mut *conn)
        .await?;

    Ok(result.rows_affected())
}

// A shared reference is an executor too.
async fn count(conn: &D1Connection) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(conn)
        .await
}

// Handlers need `Send` futures (axum, for one), even inside a Worker.
fn assert_send<T: Send>(_: T) {}

fn futures_are_send(conn: &mut D1Connection) {
    assert_send(get_user(&mut *conn, 1));
    assert_send(rename(conn, 1, "x"));
    assert_send(count(conn));
    assert_send(conn.batch([sqlx::query("DELETE FROM users")]));
}

#[test]
fn d1_is_an_sqlx_database() {
    fn assert_database<DB: sqlx::Database>() {}
    assert_database::<D1>();
    assert_eq!(<D1 as sqlx::Database>::NAME, "D1");
}
