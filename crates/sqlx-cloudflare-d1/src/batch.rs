use std::future::Future;

use futures_core::Stream;
use futures_util::{stream, TryFutureExt};
use sqlx_core::error::Error;
use sqlx_core::executor::Execute;
use sqlx_core::query::Query;
use worker::send::SendFuture;

use crate::{js, D1Arguments, D1BatchResult, D1Connection, D1QueryResult, D1};

impl D1Connection {
    /// Runs `queries` as one D1 batch: in order, in one round trip, and
    /// atomically -- if any statement fails, none of them take effect.
    ///
    /// This is the D1 answer to a transaction, which it does not support
    /// across calls (see [`D1TransactionManager`](crate::D1TransactionManager)).
    /// Returns what each statement did, in the order given; for the rows each
    /// statement returns as well, see [`fetch_batch`](Self::fetch_batch).
    ///
    /// ```no_run
    /// # async fn transfer(
    /// #     conn: &sqlx_cloudflare_d1::D1Connection,
    /// #     amount: i64,
    /// #     from: i64,
    /// #     to: i64,
    /// # ) -> Result<(), sqlx::Error> {
    /// let results = conn
    ///     .execute_batch([
    ///         sqlx::query("UPDATE accounts SET balance = balance - ?1 WHERE id = ?2")
    ///             .bind(amount)
    ///             .bind(from),
    ///         sqlx::query("UPDATE accounts SET balance = balance + ?1 WHERE id = ?2")
    ///             .bind(amount)
    ///             .bind(to),
    ///     ])
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// When binding an argument fails, or when D1 rejects any statement -- in
    /// which case the whole batch was rolled back.
    pub fn execute_batch<'q, I>(
        &self,
        queries: I,
    ) -> impl Future<Output = Result<Vec<D1QueryResult>, Error>> + Send + '_
    where
        I: IntoIterator<Item = Query<'q, D1, D1Arguments>>,
    {
        self.run_batch(queries).map_ok(|results| {
            results
                .into_iter()
                .map(|result| result.into_parts().1)
                .collect()
        })
    }

    /// Runs `queries` as one D1 batch, as
    /// [`execute_batch`](Self::execute_batch) does, and streams each
    /// statement's rows along with what it did, in the order given.
    ///
    /// The shape sqlc's `:batchexec`, `:batchone` and `:batchmany` queries
    /// take, each a stream with one item per statement -- but atomic, and in
    /// one round trip. So unlike a loop of queries it is not lazy: the whole
    /// batch runs when the stream is first polled, and a failure is the one
    /// item it yields, with nothing applied.
    ///
    /// ```no_run
    /// # async fn users(conn: &sqlx_cloudflare_d1::D1Connection) -> Result<(), sqlx::Error> {
    /// use futures_util::TryStreamExt;
    /// use sqlx::Row;
    ///
    /// let names: Vec<String> = conn
    ///     .fetch_batch([1_i64, 2, 3].map(|id| {
    ///         sqlx::query("SELECT name FROM users WHERE id = ?").bind(id)
    ///     }))
    ///     .map_ok(|result| result.rows()[0].get("name"))
    ///     .try_collect()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Rows come back keyed by name
    ///
    /// D1's `batch()` returns rows as objects, not the arrays the executor
    /// reads. Two columns with one name -- `SELECT a.id, b.id` -- collapse to
    /// the last, and a column named like an integer is ordered first. Alias
    /// such columns apart inside a batch.
    ///
    /// # Errors
    ///
    /// The stream's one item is an error when binding an argument fails, or
    /// when D1 rejects any statement -- in which case the whole batch was
    /// rolled back.
    pub fn fetch_batch<'q, I>(
        &self,
        queries: I,
    ) -> impl Stream<Item = Result<D1BatchResult, Error>> + Send + '_
    where
        I: IntoIterator<Item = Query<'q, D1, D1Arguments>>,
    {
        self.run_batch(queries)
            .map_ok(|results| stream::iter(results.into_iter().map(Ok)))
            .try_flatten_stream()
    }

    fn run_batch<'q, I>(
        &self,
        queries: I,
    ) -> impl Future<Output = Result<Vec<D1BatchResult>, Error>> + Send + '_
    where
        I: IntoIterator<Item = Query<'q, D1, D1Arguments>>,
    {
        // Taken apart here, before the future, so it holds only owned SQL and
        // arguments: the queries themselves need not outlive this call.
        let queries = queries
            .into_iter()
            .map(|mut query| {
                let arguments = query.take_arguments().map_err(Error::Encode)?;
                Ok((query.sql(), arguments.unwrap_or_default()))
            })
            .collect::<Result<Vec<_>, Error>>();

        SendFuture::new(async move {
            let queries = queries?;

            // D1 rejects an empty batch -- "No SQL statements detected" --
            // but running nothing has nothing to report, and needs no round
            // trip to find that out.
            if queries.is_empty() {
                return Ok(Vec::new());
            }

            let statements = queries
                .iter()
                .map(|(sql, arguments)| js::prepare(&self.db, sql.as_str(), arguments.values()))
                .collect::<Result<Vec<_>, _>>()?;

            js::batch(&self.db, statements).await
        })
    }
}
