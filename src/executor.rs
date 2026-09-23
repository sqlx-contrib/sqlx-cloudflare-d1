use std::future;

use futures_core::future::BoxFuture;
use futures_core::stream::BoxStream;
use futures_util::{stream, FutureExt, StreamExt, TryFutureExt};
use sqlx_core::describe::Describe;
use sqlx_core::error::Error;
use sqlx_core::executor::{Execute, Executor};
use sqlx_core::sql_str::SqlStr;
use sqlx_core::Either;
use worker::send::SendFuture;
use worker::worker_sys::types::D1PreparedStatement as D1PreparedStatementSys;

use crate::error::unsupported;
use crate::{js, D1Arguments, D1Connection, D1QueryResult, D1Row, D1Statement, D1TypeInfo, D1};

// Every future here runs JavaScript, so none of them is `Send`, and sqlx
// requires all of them to be. `SendFuture` asserts it -- soundly, because a
// Worker is single-threaded -- and it is applied here, once per method, so
// that `js` never has to know about sqlx's bounds.
impl<'c> Executor<'c> for &'c D1Connection {
    type Database = D1;

    /// Runs the query with `run()`, which reports `rows_affected` and
    /// `last_insert_rowid` but no rows.
    fn execute<'e, 'q: 'e, E>(self, query: E) -> BoxFuture<'e, Result<D1QueryResult, Error>>
    where
        'c: 'e,
        E: 'q + Execute<'q, D1>,
    {
        SendFuture::new(async move {
            let statement = prepare(self, query)?;
            js::run(&statement).await
        })
        .boxed()
    }

    fn execute_many<'e, 'q: 'e, E>(self, query: E) -> BoxStream<'e, Result<D1QueryResult, Error>>
    where
        'c: 'e,
        E: 'q + Execute<'q, D1>,
    {
        stream::once(self.execute(query)).boxed()
    }

    /// Yields the rows only, never a `D1QueryResult`: rows come from
    /// `raw({ columnNames: true })`, which reports nothing else. Use
    /// [`execute`](Executor::execute) for what a statement changed.
    fn fetch_many<'e, 'q: 'e, E>(
        self,
        query: E,
    ) -> BoxStream<'e, Result<Either<D1QueryResult, D1Row>, Error>>
    where
        'c: 'e,
        E: 'q + Execute<'q, D1>,
    {
        SendFuture::new(async move {
            let statement = prepare(self, query)?;
            js::fetch(&statement, None).await
        })
        .map_ok(|rows| stream::iter(rows.into_iter().map(|row| Ok(Either::Right(row)))))
        .try_flatten_stream()
        .boxed()
    }

    fn fetch_optional<'e, 'q: 'e, E>(self, query: E) -> BoxFuture<'e, Result<Option<D1Row>, Error>>
    where
        'c: 'e,
        E: 'q + Execute<'q, D1>,
    {
        SendFuture::new(async move {
            let statement = prepare(self, query)?;
            Ok(js::fetch(&statement, Some(1)).await?.into_iter().next())
        })
        .boxed()
    }

    fn prepare_with<'e>(
        self,
        sql: SqlStr,
        _parameters: &'e [D1TypeInfo],
    ) -> BoxFuture<'e, Result<D1Statement, Error>>
    where
        'c: 'e,
    {
        future::ready(Ok(D1Statement { sql })).boxed()
    }

    /// Always fails: D1 cannot describe a statement without running it. Only
    /// sqlx's `query!` macros need this, and they cannot target a third-party
    /// driver anyway.
    fn describe<'e>(self, _sql: SqlStr) -> BoxFuture<'e, Result<Describe<D1>, Error>>
    where
        'c: 'e,
    {
        future::ready(Err(unsupported("statements cannot be described"))).boxed()
    }
}

/// `query` prepared on `conn`'s database, with its arguments bound.
fn prepare<'q, E>(conn: &D1Connection, mut query: E) -> Result<D1PreparedStatementSys, Error>
where
    E: Execute<'q, D1>,
{
    let arguments = query.take_arguments().map_err(Error::Encode)?;
    let arguments = arguments.as_ref().map_or(&[][..], D1Arguments::values);

    js::prepare(&conn.db, query.sql().as_str(), arguments)
}

// What sqlx code actually passes -- `&mut conn` -- running on the shared
// implementation above. `&'c mut` gives up its uniqueness to become `&'c`,
// which is all a query needs.
impl<'c> Executor<'c> for &'c mut D1Connection {
    type Database = D1;

    fn execute<'e, 'q: 'e, E>(self, query: E) -> BoxFuture<'e, Result<D1QueryResult, Error>>
    where
        'c: 'e,
        E: 'q + Execute<'q, D1>,
    {
        (&*self).execute(query)
    }

    fn execute_many<'e, 'q: 'e, E>(self, query: E) -> BoxStream<'e, Result<D1QueryResult, Error>>
    where
        'c: 'e,
        E: 'q + Execute<'q, D1>,
    {
        (&*self).execute_many(query)
    }

    fn fetch_many<'e, 'q: 'e, E>(
        self,
        query: E,
    ) -> BoxStream<'e, Result<Either<D1QueryResult, D1Row>, Error>>
    where
        'c: 'e,
        E: 'q + Execute<'q, D1>,
    {
        (&*self).fetch_many(query)
    }

    fn fetch_optional<'e, 'q: 'e, E>(self, query: E) -> BoxFuture<'e, Result<Option<D1Row>, Error>>
    where
        'c: 'e,
        E: 'q + Execute<'q, D1>,
    {
        (&*self).fetch_optional(query)
    }

    fn prepare_with<'e>(
        self,
        sql: SqlStr,
        parameters: &'e [D1TypeInfo],
    ) -> BoxFuture<'e, Result<D1Statement, Error>>
    where
        'c: 'e,
    {
        (&*self).prepare_with(sql, parameters)
    }

    fn describe<'e>(self, sql: SqlStr) -> BoxFuture<'e, Result<Describe<D1>, Error>>
    where
        'c: 'e,
    {
        (&*self).describe(sql)
    }
}
