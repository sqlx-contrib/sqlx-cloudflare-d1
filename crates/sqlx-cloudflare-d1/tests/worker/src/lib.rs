//! The scenarios the integration tests run, each inside a real Worker against
//! a local D1.
//!
//! `GET /` lists the scenarios; `GET /<scenario>` runs one and answers
//! `{"ok": true}` or `{"ok": false, "error": "..."}`. The assertions live here,
//! in Rust next to the code they exercise, and `tests/driver.rs` on the host only
//! has to walk the list -- so adding a scenario is one function and one line
//! in `SCENARIOS`.

use std::future::Future;
use std::pin::Pin;

use futures_util::future::try_join;
use futures_util::{StreamExt, TryStreamExt};
use sqlx::error::ErrorKind;
use sqlx::{Connection, FromRow, Row};
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
    invalid_sql_is_an_error,
    wrong_argument_count_is_an_error,
    empty_batch,
    batch_with_unencodable_argument_runs_nothing,
    text_edge_cases,
    blob_edge_cases,
    concurrent_queries_share_a_connection,
    query_builder_bulk_insert,
    derived_types,
    fetch_batch_returns_rows_and_results,
    fetch_batch_serves_sqlc_batch_kinds,
    fetch_batch_is_atomic,
    fetch_batch_of_nothing_is_empty,
    fetch_batch_collapses_duplicate_column_names,
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
    ensure(
        row.try_get::<i64, _>("big").is_err(),
        "2^53 + 1 decoded as i64",
    )
}

async fn duplicate_column_names(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;
    let user = insert_user(conn, "dup@example.com").await?;
    let post: i64 =
        sqlx::query_scalar("INSERT INTO posts (user_id, title) VALUES (?, 't') RETURNING id")
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
        format!(
            "last_insert_rowid {:?}, id {id}",
            inserted.last_insert_rowid()
        ),
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
        (
            "INSERT INTO users (email) VALUES ('taken@example.com')",
            |k| matches!(k, ErrorKind::UniqueViolation),
        ),
        ("INSERT INTO users (email) VALUES (NULL)", |k| {
            matches!(k, ErrorKind::NotNullViolation)
        }),
        (
            "INSERT INTO users (email, age) VALUES ('neg@example.com', -1)",
            |k| matches!(k, ErrorKind::CheckViolation),
        ),
        (
            "INSERT INTO posts (user_id, title) VALUES (999999, 't')",
            |k| matches!(k, ErrorKind::ForeignKeyViolation),
        ),
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
        .execute_batch([
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
        .execute_batch([
            sqlx::query("INSERT INTO users (email) VALUES ('fresh@example.com')"),
            sqlx::query("INSERT INTO users (email) VALUES ('taken@example.com')"),
        ])
        .await;
    ensure(result.is_err(), "the batch succeeded")?;

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(conn)
        .await
        .map_err(fail)?;
    ensure(
        count == 1,
        format!("{count} users: the first insert survived"),
    )
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
    sqlx::query(
        "INSERT INTO users (email, name, age, score) VALUES ('row@example.com', 'Row', 30, 2.5)",
    )
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

// The reason `js` goes to `worker-sys` rather than `worker`: a statement D1
// rejects must come back as an error, not abort the Worker. A panic here
// would answer with an error page, which `tests/driver.rs` reports as such.
async fn invalid_sql_is_an_error(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;

    for (sql, expected) in [
        ("SELEC 1", "syntax error"),
        ("SELECT * FROM missing", "no such table"),
    ] {
        match sqlx::query(sql).fetch_all(conn).await {
            Err(sqlx::Error::Database(error)) if error.message().contains(expected) => {}
            other => return Err(format!("{sql}: {other:?}")),
        }
    }

    Ok(())
}

async fn wrong_argument_count_is_an_error(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;

    let too_few = sqlx::query("SELECT ?1, ?2")
        .bind(1_i64)
        .fetch_all(conn)
        .await;
    ensure(
        matches!(too_few, Err(sqlx::Error::Database(_))),
        format!("too few: {too_few:?}"),
    )?;

    let too_many = sqlx::query("SELECT ?")
        .bind(1_i64)
        .bind(2_i64)
        .fetch_all(conn)
        .await;
    ensure(
        matches!(too_many, Err(sqlx::Error::Database(_))),
        format!("too many: {too_many:?}"),
    )
}

async fn empty_batch(conn: &mut D1Connection) -> Outcome {
    let results = conn
        .execute_batch(std::iter::empty::<sqlx::query::Query<'_, D1, _>>())
        .await
        .map_err(fail)?;

    ensure(results.is_empty(), format!("{} results", results.len()))
}

// The encode error is caught before anything reaches D1, so not even the
// statement ahead of the bad one runs.
async fn batch_with_unencodable_argument_runs_nothing(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;

    let result = conn
        .execute_batch([
            sqlx::query("INSERT INTO users (email) VALUES ('first@example.com')"),
            sqlx::query("INSERT INTO numbers (value) VALUES (?)").bind(1_i64 << 53),
        ])
        .await;
    ensure(
        matches!(result, Err(sqlx::Error::Encode(_))),
        format!("{result:?}"),
    )?;

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(conn)
        .await
        .map_err(fail)?;
    ensure(count == 0, format!("{count} users: the first insert ran"))
}

async fn text_edge_cases(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    let astral = "🦀 𝄞 \u{0} null byte";

    let row = sqlx::query("SELECT ?1 AS empty, ?2 AS missing, ?3 AS astral")
        .bind("")
        .bind(Option::<&str>::None)
        .bind(astral)
        .fetch_one(conn)
        .await
        .map_err(fail)?;

    ensure(
        row.get::<Option<String>, _>("empty").as_deref() == Some(""),
        "the empty string came back as NULL",
    )?;
    ensure(row.get::<Option<String>, _>("missing").is_none(), "NULL")?;
    let back: String = row.get("astral");
    ensure(back == astral, format!("read back {back:?}"))
}

async fn blob_edge_cases(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;
    let large: Vec<u8> = (0..256 * 1024).map(|i| (i % 251) as u8).collect();

    for (email, bytes) in [
        ("empty@example.com", Vec::new()),
        ("large@example.com", large),
    ] {
        sqlx::query("INSERT INTO users (email, avatar) VALUES (?, ?)")
            .bind(email)
            .bind(&bytes)
            .execute(conn)
            .await
            .map_err(fail)?;

        let row = sqlx::query("SELECT avatar, typeof(avatar) AS kind FROM users WHERE email = ?")
            .bind(email)
            .fetch_one(conn)
            .await
            .map_err(fail)?;

        let kind: String = row.get("kind");
        ensure(kind == "blob", format!("{email}: stored as {kind}"))?;
        let back: Option<Vec<u8>> = row.try_get("avatar").map_err(fail)?;
        ensure(
            back.as_ref() == Some(&bytes),
            format!("{email}: read back {:?} bytes", back.map(|b| b.len())),
        )?;
    }

    Ok(())
}

// `&D1Connection` is an executor, so one connection can run queries side by
// side within a request.
async fn concurrent_queries_share_a_connection(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;

    let (a, b): (i64, String) = try_join(
        sqlx::query_scalar("SELECT 1").fetch_one(conn),
        sqlx::query_scalar("SELECT 'two'").fetch_one(conn),
    )
    .await
    .map_err(fail)?;

    ensure(a == 1 && b == "two", format!("got {a}, {b:?}"))
}

async fn query_builder_bulk_insert(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;
    let users = [
        ("qb1@example.com", 10),
        ("qb2@example.com", 20),
        ("qb3@example.com", 30),
    ];

    let mut builder = sqlx::QueryBuilder::<D1>::new("INSERT INTO users (email, age) ");
    builder.push_values(users, |mut row, (email, age)| {
        row.push_bind(email).push_bind(age);
    });
    let inserted = builder.build().execute(conn).await.map_err(fail)?;
    ensure(
        inserted.rows_affected() == 3,
        format!("rows_affected {}", inserted.rows_affected()),
    )?;

    let mut builder = sqlx::QueryBuilder::<D1>::new("SELECT SUM(age) FROM users WHERE email IN (");
    let mut emails = builder.separated(", ");
    for (email, _) in &users[..2] {
        emails.push_bind(*email);
    }
    emails.push_unseparated(")");
    let sum: i64 = builder
        .build_query_scalar()
        .fetch_one(conn)
        .await
        .map_err(fail)?;
    ensure(sum == 30, format!("sum {sum}"))
}

#[derive(Debug, Clone, Copy, PartialEq, sqlx::Type)]
#[sqlx(transparent)]
struct UserId(i64);

#[derive(Debug, Clone, Copy, PartialEq, sqlx::Type)]
#[repr(i32)]
enum Role {
    Reader = 1,
    Writer = 2,
}

#[derive(Debug, sqlx::FromRow)]
struct Member {
    id: UserId,
    role: Role,
}

async fn derived_types(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    let member = sqlx::query_as::<D1, Member>("SELECT ?1 AS id, ?2 AS role")
        .bind(UserId(5))
        .bind(Role::Writer)
        .fetch_one(conn)
        .await
        .map_err(fail)?;

    ensure(member.id == UserId(5), format!("id {:?}", member.id))?;
    ensure(
        member.role == Role::Writer,
        format!("role {:?}", member.role),
    )?;

    let reader: Role = sqlx::query_scalar("SELECT 1")
        .fetch_one(conn)
        .await
        .map_err(fail)?;
    ensure(reader == Role::Reader, format!("1 decoded as {reader:?}"))?;

    let unknown = sqlx::query_scalar::<D1, Role>("SELECT 3")
        .fetch_one(conn)
        .await;
    ensure(unknown.is_err(), format!("3 decoded: {unknown:?}"))
}

// One statement per kind of result: rows from `RETURNING`, rows from a
// `SELECT`, and a write that returns none -- each item carrying both.
async fn fetch_batch_returns_rows_and_results(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;

    let results: Vec<_> = conn
        .fetch_batch([
            sqlx::query("INSERT INTO users (email) VALUES ('bf1@example.com') RETURNING id"),
            sqlx::query("INSERT INTO users (email) VALUES ('bf2@example.com') RETURNING id"),
            sqlx::query("SELECT id, email FROM users ORDER BY id"),
            sqlx::query("UPDATE users SET name = 'x'"),
        ])
        .try_collect()
        .await
        .map_err(fail)?;

    let counts: Vec<usize> = results.iter().map(|r| r.rows().len()).collect();
    ensure(
        counts == [1, 1, 2, 0],
        format!("rows per statement {counts:?}"),
    )?;
    let changes: Vec<u64> = results.iter().map(|r| r.result().rows_affected()).collect();
    ensure(
        changes == [1, 1, 0, 2],
        format!("rows_affected {changes:?}"),
    )?;

    let first: i64 = results[0].rows()[0].get("id");
    let second: i64 = results[1].rows()[0].get("id");
    let selected: Vec<(i64, String)> = results[2]
        .rows()
        .iter()
        .map(|row| (row.get("id"), row.get("email")))
        .collect();
    ensure(
        selected
            == [
                (first, "bf1@example.com".into()),
                (second, "bf2@example.com".into()),
            ],
        format!("selected {selected:?}"),
    )
}

// What sqlc-gen-sqlx's `:batchexec`, `:batchone` and `:batchmany` would make
// of the stream: one item per input, mapped through `FromRow`.
async fn fetch_batch_serves_sqlc_batch_kinds(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;
    let alice = insert_user(conn, "alice@example.com").await?;
    let bob = insert_user(conn, "bob@example.com").await?;
    for (user, title) in [(alice, "a1"), (alice, "a2"), (bob, "b1")] {
        sqlx::query("INSERT INTO posts (user_id, title) VALUES (?, ?)")
            .bind(user)
            .bind(title)
            .execute(conn)
            .await
            .map_err(fail)?;
    }

    // :batchone
    let users: Vec<User> = conn
        .fetch_batch(
            [alice, bob].map(|id| sqlx::query("SELECT * FROM users WHERE id = ?").bind(id)),
        )
        .and_then(|result| async move {
            let row = result.rows().first().ok_or(sqlx::Error::RowNotFound)?;
            User::from_row(row)
        })
        .try_collect()
        .await
        .map_err(fail)?;
    let emails: Vec<&str> = users.iter().map(|user| user.email.as_str()).collect();
    ensure(
        emails == ["alice@example.com", "bob@example.com"],
        format!("{emails:?}"),
    )?;

    // :batchmany
    let titles: Vec<Vec<String>> = conn
        .fetch_batch([alice, bob].map(|id| {
            sqlx::query("SELECT title FROM posts WHERE user_id = ? ORDER BY title").bind(id)
        }))
        .map_ok(|result| result.rows().iter().map(|row| row.get("title")).collect())
        .try_collect()
        .await
        .map_err(fail)?;
    ensure(
        titles == [vec!["a1", "a2"], vec!["b1"]],
        format!("{titles:?}"),
    )?;

    // :batchexec
    let done: Vec<()> = conn
        .fetch_batch(
            [alice, bob].map(|id| sqlx::query("DELETE FROM posts WHERE user_id = ?").bind(id)),
        )
        .map_ok(|_| ())
        .try_collect()
        .await
        .map_err(fail)?;
    ensure(done.len() == 2, format!("{} items", done.len()))?;

    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts")
        .fetch_one(conn)
        .await
        .map_err(fail)?;
    ensure(left == 0, format!("{left} posts left"))
}

// A failure is the stream's one item, and nothing before it took effect.
async fn fetch_batch_is_atomic(conn: &mut D1Connection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;
    insert_user(conn, "taken@example.com").await?;

    let items: Vec<Result<_, sqlx::Error>> = conn
        .fetch_batch([
            sqlx::query("INSERT INTO users (email) VALUES ('fresh@example.com') RETURNING id"),
            sqlx::query("INSERT INTO users (email) VALUES ('taken@example.com')"),
        ])
        .collect()
        .await;
    ensure(items.len() == 1, format!("{} items", items.len()))?;
    match &items[0] {
        Err(sqlx::Error::Database(error)) if matches!(error.kind(), ErrorKind::UniqueViolation) => {
        }
        other => return Err(format!("{other:?}")),
    }

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(conn)
        .await
        .map_err(fail)?;
    ensure(
        count == 1,
        format!("{count} users: the first insert survived"),
    )
}

async fn fetch_batch_of_nothing_is_empty(conn: &mut D1Connection) -> Outcome {
    let items: Vec<_> = conn
        .fetch_batch(std::iter::empty::<sqlx::query::Query<'_, D1, _>>())
        .try_collect()
        .await
        .map_err(fail)?;
    ensure(items.is_empty(), format!("{} items", items.len()))
}

// Pinned rather than hidden: D1's `batch()` returns rows as objects, so two
// columns with one name collapse to the last -- unlike the executor, which
// reads arrays (see `duplicate_column_names`). If D1 ever returns arrays
// here, this fails and the documented limitation can go.
async fn fetch_batch_collapses_duplicate_column_names(conn: &mut D1Connection) -> Outcome {
    let results: Vec<_> = conn
        .fetch_batch([sqlx::query("SELECT 1 AS a, 2 AS a")])
        .try_collect()
        .await
        .map_err(fail)?;

    let row = &results[0].rows()[0];
    ensure(row.len() == 1, format!("{} columns", row.len()))?;
    ensure(row.get::<i64, _>("a") == 2, "the last `a` did not win")
}
