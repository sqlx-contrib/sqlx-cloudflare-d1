//! The scenarios the integration tests run, each inside a real SQLite-backed
//! Durable Object under `wrangler dev --local`.
//!
//! `GET /` lists the scenarios; `GET /<scenario>` runs one and answers
//! `{"ok": true}` or `{"ok": false, "error": "..."}`. The Worker forwards
//! every request to one Durable Object, which runs the scenario against its
//! own storage. The assertions live here, in Rust next to the code they
//! exercise, and `tests/driver.rs` on the host only has to walk the list -- so
//! adding a scenario is one function and one line in `SCENARIOS`.

use std::future::Future;
use std::pin::Pin;

use futures_util::future::try_join;
use futures_util::{StreamExt, TryStreamExt};
use sqlx::error::ErrorKind;
use sqlx::{Connection, FromRow, Row};
use sqlx_cloudflare_do::{Do, DoConnection};
use worker::{durable_object, event, Context, DurableObject, Env, Request, Response, State};

type Outcome = Result<(), String>;
type Scenario = for<'c> fn(&'c mut DoConnection) -> Pin<Box<dyn Future<Output = Outcome> + 'c>>;

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
    fetch_batch_keeps_duplicate_column_names,
    transaction_commits_and_returns_its_value,
    transaction_rolls_back_when_the_callback_fails,
    transaction_keeps_the_statement_error,
    transaction_reads_its_own_writes,
    select_reports_no_changes,
    begin_statement_is_refused,
];

const SCHEMA: &str = include_str!("../schema.sql");

// Every scenario in one object: they share its storage, as the D1 Worker's
// share one database, and `reset` clears what each one depends on.
// Scenarios the Worker runs itself rather than the object: they need more
// than one request to the object at a time.
const WORKER_SCENARIOS: &[&str] = &["transaction_holds_off_other_requests"];

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> worker::Result<Response> {
    let stub = env
        .durable_object("TEST")?
        .id_from_name("scenarios")?
        .get_stub()?;

    match req.path().as_str() {
        "/" => {
            let mut names: Vec<String> = stub.fetch_with_request(req).await?.json().await?;
            names.extend(WORKER_SCENARIOS.iter().map(|name| (*name).to_owned()));
            Response::from_json(&names)
        }
        "/transaction_holds_off_other_requests" => {
            let body = match transaction_holds_off_other_requests(&env).await {
                Ok(()) => serde_json::json!({ "ok": true }),
                Err(error) => serde_json::json!({ "ok": false, "error": error }),
            };
            Response::from_json(&body)
        }
        _ => stub.fetch_with_request(req).await,
    }
}

#[durable_object]
pub struct TestObject {
    state: State,
}

impl DurableObject for TestObject {
    fn new(state: State, _env: Env) -> Self {
        Self { state }
    }

    async fn fetch(&self, req: Request) -> worker::Result<Response> {
        let name = req.path().trim_start_matches('/').to_owned();

        if name.is_empty() {
            let names: Vec<_> = SCENARIOS.iter().map(|(name, _)| *name).collect();
            return Response::from_json(&names);
        }

        // The steps of the Worker-level scenarios, which are not scenarios
        // themselves.
        if let Some(op) = name.strip_prefix("op/") {
            let conn = DoConnection::new(self.state.storage());
            conn.sql().exec(SCHEMA, None)?;
            return op_step(&conn, op).await;
        }

        let Some((_, scenario)) = SCENARIOS.iter().find(|(n, _)| *n == name) else {
            return Response::error(format!("no scenario `{name}`"), 404);
        };

        // A fresh connection per request, from the object's own storage:
        // `begin_fails` needs one it can borrow mutably.
        let mut conn = DoConnection::new(self.state.storage());
        conn.sql().exec(SCHEMA, None)?;

        let body = match scenario(&mut conn).await {
            Ok(()) => serde_json::json!({ "ok": true }),
            Err(error) => serde_json::json!({ "ok": false, "error": error }),
        };

        Response::from_json(&body)
    }
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

async fn reset(conn: &DoConnection) -> Outcome {
    for table in ["posts", "users", "numbers"] {
        sqlx::query(sqlx::AssertSqlSafe(format!("DELETE FROM {table}")))
            .execute(conn)
            .await
            .map_err(fail)?;
    }
    Ok(())
}

async fn insert_user(conn: &DoConnection, email: &str) -> Result<i64, String> {
    sqlx::query_scalar("INSERT INTO users (email, name) VALUES (?, ?) RETURNING id")
        .bind(email)
        .bind("someone")
        .fetch_one(conn)
        .await
        .map_err(fail)
}

async fn select_literals(conn: &mut DoConnection) -> Outcome {
    let conn = &*conn;
    let row = sqlx::query("SELECT 1 AS n, 'x' AS s")
        .fetch_one(conn)
        .await
        .map_err(fail)?;

    ensure(row.get::<i64, _>("n") == 1, "n")?;
    ensure(row.get::<String, _>("s") == "x", "s")
}

async fn bind_round_trip(conn: &mut DoConnection) -> Outcome {
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

async fn null_values(conn: &mut DoConnection) -> Outcome {
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

async fn whole_real_decodes_as_f64(conn: &mut DoConnection) -> Outcome {
    let conn = &*conn;
    let value: f64 = sqlx::query_scalar("SELECT 3.0")
        .fetch_one(conn)
        .await
        .map_err(fail)?;

    ensure((value - 3.0).abs() < f64::EPSILON, format!("got {value}"))
}

// Open question 1: which JavaScript representation Do stores as a BLOB, and
// what it hands back.
async fn blob_round_trip(conn: &mut DoConnection) -> Outcome {
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

async fn integer_bounds(conn: &mut DoConnection) -> Outcome {
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

async fn duplicate_column_names(conn: &mut DoConnection) -> Outcome {
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

async fn empty_result(conn: &mut DoConnection) -> Outcome {
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

async fn fetch_optional_takes_the_first_row(conn: &mut DoConnection) -> Outcome {
    let conn = &*conn;
    let n: Option<i64> = sqlx::query_scalar("SELECT 1 UNION ALL SELECT 2 ORDER BY 1")
        .fetch_optional(conn)
        .await
        .map_err(fail)?;

    ensure(n == Some(1), format!("got {n:?}"))
}

async fn execute_reports_changes(conn: &mut DoConnection) -> Outcome {
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

async fn constraint_errors(conn: &mut DoConnection) -> Outcome {
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

async fn batch_reports_each_statement(conn: &mut DoConnection) -> Outcome {
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

async fn batch_is_atomic(conn: &mut DoConnection) -> Outcome {
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

async fn begin_fails(conn: &mut DoConnection) -> Outcome {
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

async fn from_row(conn: &mut DoConnection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;
    sqlx::query(
        "INSERT INTO users (email, name, age, score) VALUES ('row@example.com', 'Row', 30, 2.5)",
    )
    .execute(conn)
    .await
    .map_err(fail)?;

    let user = sqlx::query_as::<Do, User>("SELECT * FROM users")
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

// The reason `js` goes to `worker-sys` rather than `worker`: a statement Do
// rejects must come back as an error, not abort the Worker. A panic here
// would answer with an error page, which `tests/driver.rs` reports as such.
async fn invalid_sql_is_an_error(conn: &mut DoConnection) -> Outcome {
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

async fn wrong_argument_count_is_an_error(conn: &mut DoConnection) -> Outcome {
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

async fn empty_batch(conn: &mut DoConnection) -> Outcome {
    let results = conn
        .execute_batch(std::iter::empty::<sqlx::query::Query<'_, Do, _>>())
        .await
        .map_err(fail)?;

    ensure(results.is_empty(), format!("{} results", results.len()))
}

// The encode error is caught before anything reaches Do, so not even the
// statement ahead of the bad one runs.
async fn batch_with_unencodable_argument_runs_nothing(conn: &mut DoConnection) -> Outcome {
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

async fn text_edge_cases(conn: &mut DoConnection) -> Outcome {
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

async fn blob_edge_cases(conn: &mut DoConnection) -> Outcome {
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

// `&DoConnection` is an executor, so one connection can run queries side by
// side within a request.
async fn concurrent_queries_share_a_connection(conn: &mut DoConnection) -> Outcome {
    let conn = &*conn;

    let (a, b): (i64, String) = try_join(
        sqlx::query_scalar("SELECT 1").fetch_one(conn),
        sqlx::query_scalar("SELECT 'two'").fetch_one(conn),
    )
    .await
    .map_err(fail)?;

    ensure(a == 1 && b == "two", format!("got {a}, {b:?}"))
}

async fn query_builder_bulk_insert(conn: &mut DoConnection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;
    let users = [
        ("qb1@example.com", 10),
        ("qb2@example.com", 20),
        ("qb3@example.com", 30),
    ];

    let mut builder = sqlx::QueryBuilder::<Do>::new("INSERT INTO users (email, age) ");
    builder.push_values(users, |mut row, (email, age)| {
        row.push_bind(email).push_bind(age);
    });
    let inserted = builder.build().execute(conn).await.map_err(fail)?;
    ensure(
        inserted.rows_affected() == 3,
        format!("rows_affected {}", inserted.rows_affected()),
    )?;

    let mut builder = sqlx::QueryBuilder::<Do>::new("SELECT SUM(age) FROM users WHERE email IN (");
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

async fn derived_types(conn: &mut DoConnection) -> Outcome {
    let conn = &*conn;
    let member = sqlx::query_as::<Do, Member>("SELECT ?1 AS id, ?2 AS role")
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

    let unknown = sqlx::query_scalar::<Do, Role>("SELECT 3")
        .fetch_one(conn)
        .await;
    ensure(unknown.is_err(), format!("3 decoded: {unknown:?}"))
}

// `changes()` keeps the count of the last write, so a statement that wrote
// nothing must not report the one before it.
async fn select_reports_no_changes(conn: &mut DoConnection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;
    insert_user(conn, "stale@example.com").await?;

    let selected = sqlx::query("SELECT * FROM users")
        .execute(conn)
        .await
        .map_err(fail)?;
    ensure(
        selected.rows_affected() == 0,
        format!("SELECT rows_affected {}", selected.rows_affected()),
    )?;
    ensure(
        selected.last_insert_rowid().is_none(),
        format!(
            "SELECT last_insert_rowid {:?}",
            selected.last_insert_rowid()
        ),
    )?;

    let ignored = sqlx::query(
        "INSERT INTO users (email) VALUES ('stale@example.com') ON CONFLICT DO NOTHING",
    )
    .execute(conn)
    .await
    .map_err(fail)?;
    ensure(
        ignored.rows_affected() == 0,
        format!("ignored INSERT rows_affected {}", ignored.rows_affected()),
    )
}

// What `DoTransactionManager` refuses on the driver's side, `sql.exec` refuses
// on its own: there is no way round it by sending `BEGIN` as a query.
async fn begin_statement_is_refused(conn: &mut DoConnection) -> Outcome {
    let result = sqlx::query("BEGIN TRANSACTION").execute(&*conn).await;
    ensure(result.is_err(), format!("BEGIN ran: {result:?}"))
}

// One statement per kind of result: rows from `RETURNING`, rows from a
// `SELECT`, and a write that returns none -- each item carrying both.
async fn fetch_batch_returns_rows_and_results(conn: &mut DoConnection) -> Outcome {
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
async fn fetch_batch_serves_sqlc_batch_kinds(conn: &mut DoConnection) -> Outcome {
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
async fn fetch_batch_is_atomic(conn: &mut DoConnection) -> Outcome {
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

async fn fetch_batch_of_nothing_is_empty(conn: &mut DoConnection) -> Outcome {
    let items: Vec<_> = conn
        .fetch_batch(std::iter::empty::<sqlx::query::Query<'_, Do, _>>())
        .try_collect()
        .await
        .map_err(fail)?;
    ensure(items.is_empty(), format!("{} items", items.len()))
}

// Durable Object storage reads every statement's cursor as arrays, batch or
// not, so -- unlike D1's batch -- both columns survive.
async fn fetch_batch_keeps_duplicate_column_names(conn: &mut DoConnection) -> Outcome {
    let results: Vec<_> = conn
        .fetch_batch([sqlx::query("SELECT 1 AS a, 2 AS a")])
        .try_collect()
        .await
        .map_err(fail)?;

    let row = &results[0].rows()[0];
    ensure(row.len() == 2, format!("{} columns", row.len()))?;
    ensure(
        row.get::<i64, _>(0) == 1 && row.get::<i64, _>(1) == 2,
        "values",
    )
}

// ---- Interactive transactions: `DoConnection::transaction`.

async fn transaction_commits_and_returns_its_value(conn: &mut DoConnection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;

    let id = conn
        .transaction(|tx| async move {
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO users (email) VALUES ('tx@example.com') RETURNING id",
            )
            .fetch_one(&tx)
            .await?;
            sqlx::query("INSERT INTO posts (user_id, title) VALUES (?, 'first')")
                .bind(id)
                .execute(&tx)
                .await?;
            Ok::<_, sqlx::Error>(id)
        })
        .await
        .map_err(fail)?;

    let owner: i64 = sqlx::query_scalar("SELECT user_id FROM posts WHERE title = 'first'")
        .fetch_one(conn)
        .await
        .map_err(fail)?;
    ensure(owner == id, format!("post owned by {owner}, user {id}"))
}

#[derive(Debug)]
#[allow(
    dead_code,
    reason = "the payload is read through `Debug`, when a scenario reports it"
)]
enum AppError {
    Refused,
    Sqlx(sqlx::Error),
}

impl From<sqlx::Error> for AppError {
    fn from(error: sqlx::Error) -> Self {
        AppError::Sqlx(error)
    }
}

// The caller's own error type comes back as the callback returned it, and
// what the callback wrote is gone.
async fn transaction_rolls_back_when_the_callback_fails(conn: &mut DoConnection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;

    let result = conn
        .transaction(|tx| async move {
            sqlx::query("INSERT INTO users (email) VALUES ('undone@example.com')")
                .execute(&tx)
                .await?;
            Err::<(), _>(AppError::Refused)
        })
        .await;
    ensure(
        matches!(result, Err(AppError::Refused)),
        format!("{result:?}"),
    )?;

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(conn)
        .await
        .map_err(fail)?;
    ensure(count == 0, format!("{count} users: the insert survived"))
}

// A statement failing inside is the caller's error, `ErrorKind` and all,
// and the statement before it is rolled back.
async fn transaction_keeps_the_statement_error(conn: &mut DoConnection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;
    insert_user(conn, "taken@example.com").await?;

    let result = conn
        .transaction(|tx| async move {
            sqlx::query("INSERT INTO users (email) VALUES ('fresh@example.com')")
                .execute(&tx)
                .await?;
            sqlx::query("INSERT INTO users (email) VALUES ('taken@example.com')")
                .execute(&tx)
                .await?;
            Ok::<_, sqlx::Error>(())
        })
        .await;
    match &result {
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

// What a batch cannot do: read a write back, and decide on it, before the
// transaction ends.
async fn transaction_reads_its_own_writes(conn: &mut DoConnection) -> Outcome {
    let conn = &*conn;
    reset(conn).await?;

    let (inside, total) = conn
        .transaction(|tx| async move {
            for email in ["r1@example.com", "r2@example.com"] {
                sqlx::query("INSERT INTO users (email) VALUES (?)")
                    .bind(email)
                    .execute(&tx)
                    .await?;
            }
            let inside: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
                .fetch_one(&tx)
                .await?;
            let updated = sqlx::query("UPDATE users SET age = ?")
                .bind(inside)
                .execute(&tx)
                .await?;
            Ok::<_, sqlx::Error>((inside, updated.rows_affected()))
        })
        .await
        .map_err(fail)?;
    ensure(
        inside == 2 && total == 2,
        format!("saw {inside}, updated {total}"),
    )?;

    let ages: Vec<i64> = sqlx::query_scalar("SELECT age FROM users")
        .fetch_all(conn)
        .await
        .map_err(fail)?;
    ensure(ages == [2, 2], format!("ages {ages:?}"))
}

// ---- Worker-level: a second request while a transaction is open.

// The object delivers no other request while a transaction runs, even while
// its callback awaits a timer -- which is what keeps a concurrent request's
// writes out of it, and out of its rollback. Measured in #7; pinned here.
async fn transaction_holds_off_other_requests(env: &Env) -> Outcome {
    let step = |op: &'static str| async move {
        step_on(env, op)
            .await
            .map_err(|error| format!("step `{op}`: {error}"))
    };

    step("reset").await?;
    let other = async {
        worker::Delay::from(std::time::Duration::from_millis(50)).await;
        step("insert").await
    };
    let (held, other) = futures_util::future::try_join(step("hold"), other).await?;
    let after = step("emails").await?;

    ensure(held["rolled_back"] == true, format!("hold: {held}"))?;
    ensure(
        held["seen_inside"] == serde_json::json!(["held"]),
        format!("the transaction saw {}", held["seen_inside"]),
    )?;
    let (ended, at) = (held["ended"].as_f64(), other["at"].as_f64());
    ensure(
        matches!((ended, at), (Some(ended), Some(at)) if at >= ended),
        format!("the insert ran at {at:?}, before the transaction ended at {ended:?}"),
    )?;
    ensure(
        after == serde_json::json!(["other"]),
        format!("left {after}: the rollback took the other request's write, or kept its own"),
    )
}

// One step of a Worker-level scenario, on its own object so it cannot
// disturb the object the other scenarios share.
async fn step_on(env: &Env, op: &str) -> worker::Result<serde_json::Value> {
    let namespace = env.durable_object("TEST")?;
    let stub = namespace.id_from_name("concurrency")?.get_stub()?;
    let mut response = stub.fetch_with_str(&format!("https://do/op/{op}")).await?;
    response.json().await
}

// How the `hold` step's callback ends: with what it saw, which rolls the
// transaction back -- or with a query's error, which is a failed step.
#[derive(Debug)]
#[allow(
    dead_code,
    reason = "the payload is read through `Debug`, when a scenario reports it"
)]
enum HoldError {
    Held(Vec<String>),
    Sqlx(sqlx::Error),
}

impl From<sqlx::Error> for HoldError {
    fn from(error: sqlx::Error) -> Self {
        HoldError::Sqlx(error)
    }
}

async fn op_step(conn: &DoConnection, op: &str) -> worker::Result<Response> {
    let now = || worker::Date::now().as_millis();

    let body = match op {
        "reset" => {
            reset(conn).await.map_err(worker::Error::RustError)?;
            serde_json::json!({})
        }
        "emails" => {
            let emails: Vec<String> = sqlx::query_scalar("SELECT email FROM users ORDER BY id")
                .fetch_all(conn)
                .await
                .map_err(|error| worker::Error::RustError(error.to_string()))?;
            serde_json::json!(emails)
        }
        "insert" => {
            let at = now();
            sqlx::query("INSERT INTO users (email) VALUES ('other')")
                .execute(conn)
                .await
                .map_err(|error| worker::Error::RustError(error.to_string()))?;
            serde_json::json!({ "at": at })
        }
        "hold" => {
            let result = conn
                .transaction(|tx| async move {
                    sqlx::query("INSERT INTO users (email) VALUES ('held')")
                        .execute(&tx)
                        .await?;
                    worker::Delay::from(std::time::Duration::from_millis(300)).await;
                    let seen: Vec<String> =
                        sqlx::query_scalar("SELECT email FROM users ORDER BY id")
                            .fetch_all(&tx)
                            .await?;
                    Err::<(), _>(HoldError::Held(seen))
                })
                .await;
            let seen = match result {
                Err(HoldError::Held(seen)) => seen,
                other => return Response::error(format!("{other:?}"), 500),
            };
            serde_json::json!({ "rolled_back": true, "seen_inside": seen, "ended": now() })
        }
        _ => return Response::error(format!("no step `{op}`"), 404),
    };

    Response::from_json(&body)
}
