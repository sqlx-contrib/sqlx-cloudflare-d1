use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;

use sqlx_core::error::Error;
use sqlx_core::executor::Execute;
use sqlx_core::query::Query;
use worker::send::SendFuture;

use crate::{js, Do, DoArguments, DoConnection, DoQueryResult};

impl DoConnection {
    /// Runs `queries` in order and atomically: if any statement fails, none
    /// of them take effect.
    ///
    /// This is the Durable Object answer to a transaction, which `sql.exec`
    /// does not allow (see [`DoTransactionManager`](crate::DoTransactionManager)).
    /// The statements run inside `Storage::transaction`, and a failing one
    /// rolls the others back. Returns what each statement did, in the order
    /// given; rows a statement returns are not reported.
    ///
    /// ```no_run
    /// # async fn transfer(
    /// #     conn: &sqlx_cloudflare_do::DoConnection,
    /// #     amount: i64,
    /// #     from: i64,
    /// #     to: i64,
    /// # ) -> Result<(), sqlx::Error> {
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
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// When binding an argument fails, in which case nothing ran, or when any
    /// statement fails -- in which case the whole batch was rolled back.
    pub fn batch<'q, I>(
        &self,
        queries: I,
    ) -> impl Future<Output = Result<Vec<DoQueryResult>, Error>> + Send + '_
    where
        I: IntoIterator<Item = Query<'q, Do, DoArguments>>,
    {
        // Taken apart here, before the future, so it holds only owned SQL and
        // arguments: the queries themselves need not outlive this call, and
        // the transaction's callback has to own what it runs.
        let queries = queries
            .into_iter()
            .map(|mut query| {
                let arguments = query.take_arguments().map_err(Error::Encode)?;
                Ok((query.sql(), arguments.unwrap_or_default()))
            })
            .collect::<Result<Vec<_>, Error>>();

        // `Storage::transaction` and its callback's future hold JavaScript
        // handles, so neither is `Send`; `SendFuture` asserts it, soundly,
        // because a Durable Object runs on one thread.
        SendFuture::new(async move {
            let queries = queries?;

            if queries.is_empty() {
                return Ok(Vec::new());
            }

            // The callback reports through this rather than its return value,
            // which `worker` flattens to a string: the statement's own error,
            // with its `ErrorKind`, is what the caller should see.
            let outcome = Rc::new(RefCell::new(None));
            let slot = Rc::clone(&outcome);
            let sql = self.sql.clone();

            let committed = self
                .storage
                .transaction(move |_| async move {
                    let results = queries
                        .iter()
                        .map(|(query, arguments)| js::run(&sql, query.as_str(), arguments.values()))
                        .collect::<Result<Vec<_>, Error>>();
                    let failed = results.is_err();
                    *slot.borrow_mut() = Some(results);

                    if failed {
                        // Any error will do: rejecting is what rolls back.
                        Err(worker::Error::RustError("a statement failed".into()))
                    } else {
                        Ok(())
                    }
                })
                .await;

            let outcome = outcome.borrow_mut().take();
            match (committed, outcome) {
                (_, Some(Err(error))) => Err(error),
                (Ok(()), Some(Ok(results))) => Ok(results),
                (Err(error), _) => Err(Error::Protocol(format!(
                    "the Durable Object storage transaction failed: {error}"
                ))),
                (Ok(()), None) => Err(Error::Protocol(
                    "the Durable Object storage transaction never ran its callback".into(),
                )),
            }
        })
    }
}
