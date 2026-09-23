use std::future::Future;

use sqlx_core::error::Error;
use sqlx_core::executor::Execute;
use sqlx_core::query::Query;
use worker::send::SendFuture;

use crate::{js, D1Arguments, D1Connection, D1QueryResult, D1};

impl D1Connection {
    /// Runs `queries` as one D1 batch: in order, in one round trip, and
    /// atomically -- if any statement fails, none of them take effect.
    ///
    /// This is the D1 answer to a transaction, which it does not support
    /// across calls (see [`D1TransactionManager`](crate::D1TransactionManager)).
    /// Returns what each statement did, in the order given; rows a statement
    /// returns are not reported.
    ///
    /// ```ignore
    /// let results = conn
    ///     .batch([
    ///         sqlx::query("UPDATE accounts SET balance = balance - ?1 WHERE id = ?2")
    ///             .bind(amount)
    ///             .bind(from),
    ///         sqlx::query("UPDATE accounts SET balance = balance + ?1 WHERE id = ?2")
    ///             .bind(amount)
    ///             .bind(to),
    ///     ])
    ///     .await?;
    /// ```
    ///
    /// # Errors
    ///
    /// When binding an argument fails, or when D1 rejects any statement -- in
    /// which case the whole batch was rolled back.
    pub fn batch<'q, I>(
        &self,
        queries: I,
    ) -> impl Future<Output = Result<Vec<D1QueryResult>, Error>> + Send + '_
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
            let statements = queries?
                .iter()
                .map(|(sql, arguments)| js::prepare(&self.db, sql.as_str(), arguments.values()))
                .collect::<Result<Vec<_>, _>>()?;

            js::batch(&self.db, statements).await
        })
    }
}
