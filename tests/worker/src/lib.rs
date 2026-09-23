//! The scenarios the integration tests run, each inside a real Worker against
//! a local D1.
//!
//! `GET /` lists the scenarios; `GET /<scenario>` runs one and answers
//! `{"ok": true}` or `{"ok": false, "error": "..."}`. The assertions live here,
//! in Rust next to the code they exercise, and `tests/d1.rs` on the host only
//! has to walk the list -- so adding a scenario is one function and one line
//! in `SCENARIOS`.

use std::future::Future;
use std::pin::Pin;

use sqlx::error::ErrorKind;
use sqlx::{Connection, Row};
use sqlx_cloudflare_d1::{D1Connection, D1};
use worker::{event, Context, Env, Request, Response};

type Outcome = Result<(), String>;
type Scenario = for<'c> fn(&'c mut D1Connection) -> Pin<Box<dyn Future<Output = Outcome> + 'c>>;

macro_rules! scenarios {
    ($($name:ident),* $(,)?) => {
        const SCENARIOS: &[(&str, Scenario)] = &[
            $((stringify!($name), |conn| Box::pin($name(conn)))),*
        ];
    };
}

scenarios![
    select_literals,
    bind_round_trip,
    null_values,
    whole_real_decodes_as_f64,
    blob_round_trip,
    integer_bounds,
    duplicate_column_names,
    empty_result,
    fetch_optional_takes_the_first_row,
    execute_reports_changes,
    constraint_errors,
    batch_reports_each_statement,
    batch_is_atomic,
    begin_fails,
    from_row,
];

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> worker::Result<Response> {
    let name = req.path().trim_start_matches('/').to_owned();

    if name.is_empty() {
        let names: Vec<_> = SCENARIOS.iter().map(|(name, _)| *name).collect();
        return Response::from_json(&names);
    }

    let Some((_, scenario)) = SCENARIOS.iter().find(|(n, _)| *n == name) else {
        return Response::error(format!("no scenario `{name}`"), 404);
    };

    let mut conn = D1Connection::from_env(&env, "DB")?;
    let body = match scenario(&mut conn).await {
        Ok(()) => serde_json::json!({ "ok": true }),
        Err(error) => serde_json::json!({ "ok": false, "error": error }),
    };

    Response::from_json(&body)
}

/// `Err` with the message unless `condition` holds.
fn ensure(condition: bool, message: impl Into<String>) -> Outcome {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

fn fail(error: sqlx::Error) -> String {
    error.to_string()
}

async fn reset(conn: &D1Connection) -> Outcome {
    for table in ["posts", "users", "numbers"] {
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table}")))
            .execute(conn)
            .await
            .map_err(fail)?;
    }
    Ok(())
}

async fn insert_user(conn: &D1Connection, email: &str) -> Result<i64, String> {
    sqlx::query_scalar("INSERT INTO users (email, name) VALUES (?, ?) RETURNING id")
        .bind(email)
        .bind("someone")
        .fetch_one(conn)
        .await
        .map_err(fail)
}

async fn select_literals(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    let row = sqlx::query("SELECT 1 AS n, 'x' AS s")
        .fetch_one(conn)
        .await
        .map_err(fail)?;

    ensure(row.get::<i64, _>("n") == 1, "n")?;
    ensure(row.get::<String, _>("s") == "x", "s")
}

async fn bind_round_trip(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    let row = sqlx::query("SELECT ?1 AS i, ?2 AS r, ?3 AS t, ?4 AS b, ?5 AS small")
        .bind(42_i64)
        .bind(1.5_f64)
        .bind("héllo, wörld")
        .bind(true)
        .bind(-7_i16)
        .fetch_one(conn)
        .await
        .map_err(fail)?;

    ensure(row.get::<i64, _>("i") == 42, "i64")?;
    ensure((row.get::<f64, _>("r") - 1.5).abs() < f64::EPSILON, "f64")?;
    ensure(row.get::<&str, _>("t") == "héllo, wörld", "text")?;
    ensure(row.get::<bool, _>("b"), "bool")?;
    ensure(row.get::<i16, _>("small") == -7, "i16")
}

async fn null_values(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    let row = sqlx::query("SELECT NULL AS a, ?1 AS b")
        .bind(Option::<String>::None)
        .fetch_one(conn)
        .await
        .map_err(fail)?;

    ensure(row.get::<Option<String>, _>("a").is_none(), "literal NULL")?;
    ensure(row.get::<Option<i64>, _>("b").is_none(), "bound NULL")?;
    ensure(row.try_get::<i64, _>("a").is_err(), "NULL into i64")
}

async fn whole_real_decodes_as_f64(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    let value: f64 = sqlx::query_scalar("SELECT 3.0")
        .fetch_one(conn)
        .await
        .map_err(fail)?;

    ensure((value - 3.0).abs() < f64::EPSILON, format!("got {value}"))
}

// Open question 1: which JavaScript representation D1 stores as a BLOB, and
// what it hands back.
async fn blob_round_trip(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;
    let bytes = vec![0_u8, 1, 127, 128, 255];

    sqlx::query("INSERT INTO users (email, avatar) VALUES ('blob@example.com', ?)")
        .bind(&bytes)
        .execute(conn)
        .await
        .map_err(fail)?;

    let row = sqlx::query("SELECT avatar, typeof(avatar) AS kind FROM users")
        .fetch_one(conn)
        .await
        .map_err(fail)?;

    let kind: String = row.get("kind");
    ensure(kind == "blob", format!("stored as {kind}"))?;
    let back: Vec<u8> = row.try_get("avatar").map_err(fail)?;
    ensure(back == bytes, format!("read back {back:?}"))
}

async fn integer_bounds(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;
    const MAX: i64 = (1 << 53) - 1;

    for value in [MAX, -MAX] {
        sqlx::query("INSERT INTO numbers (value) VALUES (?)")
            .bind(value)
            .execute(conn)
            .await
            .map_err(fail)?;
    }

    let values: Vec<i64> = sqlx::query_scalar("SELECT value FROM numbers ORDER BY value")
        .fetch_all(conn)
        .await
        .map_err(fail)?;
    ensure(values == [-MAX, MAX], format!("read back {values:?}"))?;

    let refused = sqlx::query("SELECT ?").bind(MAX + 1).execute(conn).await;
    ensure(
        matches!(refused, Err(sqlx::Error::Encode(_))),
        format!("binding 2^53: {refused:?}"),
    )?;

    // Beyond 2^53 on the database side: comes back as a REAL, which must not
    // decode as i64.
    let row = sqlx::query("SELECT 9007199254740993 AS big")
        .fetch_one(conn)
        .await
        .map_err(fail)?;
    ensure(row.try_get::<i64, _>("big").is_err(), "2^53 + 1 decoded as i64")
}

async fn duplicate_column_names(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;
    let user = insert_user(conn, "dup@example.com").await?;
    let post: i64 = sqlx::query_scalar("INSERT INTO posts (user_id, title) VALUES (?, 't') RETURNING id")
        .bind(user)
        .fetch_one(conn)
        .await
        .map_err(fail)?;

    let row = sqlx::query("SELECT u.id, p.id FROM users u JOIN posts p ON p.user_id = u.id")
        .fetch_one(conn)
        .await
        .map_err(fail)?;

    ensure(row.len() == 2, format!("{} columns", row.len()))?;
    ensure(row.get::<i64, _>(0) == user, "first id")?;
    ensure(row.get::<i64, _>(1) == post, "second id")?;
    // As sqlx-sqlite: the name finds the last column that has it.
    ensure(row.get::<i64, _>("id") == post, "id by name")
}

async fn empty_result(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;

    let rows = sqlx::query("SELECT id FROM users")
        .fetch_all(conn)
        .await
        .map_err(fail)?;
    ensure(rows.is_empty(), "fetch_all")?;

    let row = sqlx::query("SELECT id FROM users")
        .fetch_optional(conn)
        .await
        .map_err(fail)?;
    ensure(row.is_none(), "fetch_optional")?;

    let missing = sqlx::query("SELECT id FROM users").fetch_one(conn).await;
    ensure(
        matches!(missing, Err(sqlx::Error::RowNotFound)),
        "fetch_one",
    )
}

async fn fetch_optional_takes_the_first_row(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    let n: Option<i64> = sqlx::query_scalar("SELECT 1 UNION ALL SELECT 2 ORDER BY 1")
        .fetch_optional(conn)
        .await
        .map_err(fail)?;

    ensure(n == Some(1), format!("got {n:?}"))
}

async fn execute_reports_changes(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;

    let inserted = sqlx::query("INSERT INTO users (email) VALUES ('meta@example.com')")
        .execute(conn)
        .await
        .map_err(fail)?;
    let id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE email = 'meta@example.com'")
        .fetch_one(conn)
        .await
        .map_err(fail)?;

    ensure(inserted.rows_affected() == 1, "insert rows_affected")?;
    ensure(
        inserted.last_insert_rowid() == Some(id),
        format!("last_insert_rowid {:?}, id {id}", inserted.last_insert_rowid()),
    )?;

    insert_user(conn, "meta2@example.com").await?;
    let updated = sqlx::query("UPDATE users SET name = 'n'")
        .execute(conn)
        .await
        .map_err(fail)?;
    ensure(
        updated.rows_affected() == 2,
        format!("update rows_affected {}", updated.rows_affected()),
    )
}

async fn constraint_errors(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;
    insert_user(conn, "taken@example.com").await?;

    let cases: [(&str, fn(&ErrorKind) -> bool); 4] = [
        ("INSERT INTO users (email) VALUES ('taken@example.com')", |k| {
            matches!(k, ErrorKind::UniqueViolation)
        }),
        ("INSERT INTO users (email) VALUES (NULL)", |k| {
            matches!(k, ErrorKind::NotNullViolation)
        }),
        ("INSERT INTO users (email, age) VALUES ('neg@example.com', -1)", |k| {
            matches!(k, ErrorKind::CheckViolation)
        }),
        ("INSERT INTO posts (user_id, title) VALUES (999999, 't')", |k| {
            matches!(k, ErrorKind::ForeignKeyViolation)
        }),
    ];

    for (sql, expected) in cases {
        match sqlx::query(sql).execute(conn).await {
            Err(sqlx::Error::Database(error)) if expected(&error.kind()) => {}
            Err(sqlx::Error::Database(error)) => {
                return Err(format!(
                    "{sql}: kind {:?} for message {:?}",
                    error.kind(),
                    error.message()
                ))
            }
            other => return Err(format!("{sql}: {other:?}")),
        }
    }

    Ok(())
}

async fn batch_reports_each_statement(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;

    let results = conn
        .batch([
            sqlx::query("INSERT INTO users (email) VALUES (?)").bind("b1@example.com"),
            sqlx::query("INSERT INTO users (email) VALUES (?)").bind("b2@example.com"),
            sqlx::query("UPDATE users SET name = 'x'"),
        ])
        .await
        .map_err(fail)?;

    let changes: Vec<u64> = results.iter().map(|r| r.rows_affected()).collect();
    ensure(changes == [1, 1, 2], format!("rows_affected {changes:?}"))
}

async fn batch_is_atomic(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;
    insert_user(conn, "taken@example.com").await?;

    let result = conn
        .batch([
            sqlx::query("INSERT INTO users (email) VALUES ('fresh@example.com')"),
            sqlx::query("INSERT INTO users (email) VALUES ('taken@example.com')"),
        ])
        .await;
    ensure(result.is_err(), "the batch succeeded")?;

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(conn)
        .await
        .map_err(fail)?;
    ensure(count == 1, format!("{count} users: the first insert survived"))
}

async fn begin_fails(conn: &mut D1Connection) -> Outcome {
    ensure(conn.begin().await.is_err(), "begin succeeded")
}

#[derive(Debug, sqlx::FromRow)]
struct User {
    id: i64,
    email: String,
    name: Option<String>,
    age: Option<i32>,
    score: Option<f64>,
    active: bool,
    avatar: Option<Vec<u8>>,
}

async fn from_row(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;
    sqlx::query("INSERT INTO users (email, name, age, score) VALUES ('row@example.com', 'Row', 30, 2.5)")
        .execute(conn)
        .await
        .map_err(fail)?;

    let user = sqlx::query_as::<D1, User>("SELECT * FROM users")
        .fetch_one(conn)
        .await
        .map_err(fail)?;

    ensure(user.email == "row@example.com", "email")?;
    ensure(user.name.as_deref() == Some("Row"), "name")?;
    ensure(user.age == Some(30), "age")?;
    ensure(user.score == Some(2.5), "score")?;
    ensure(user.active, "active")?;
    ensure(user.avatar.is_none(), "avatar")?;
    ensure(user.id > 0, "id")
}
