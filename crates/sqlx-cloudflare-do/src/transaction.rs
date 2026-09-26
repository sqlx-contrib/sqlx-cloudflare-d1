use std::cell::RefCell;
use std::fmt::{self, Debug, Formatter};
use std::future::Future;
use std::mem;
use std::rc::Rc;

use futures_channel::oneshot;
use futures_util::future::{self, Either, FutureExt, Shared};
use sqlx_core::error::Error;
use sqlx_core::sql_str::SqlStr;
use sqlx_core::transaction::TransactionManager;
use worker::send::SendFuture;

use crate::error::unsupported;
use crate::{Do, DoConnection};

/// Why a parked transaction rejects: a rollback, asked for. Any error would
/// do -- rejecting is what rolls back -- but this one is recognisable.
const ROLLED_BACK: &str = "rolled back";

const ALREADY_OPEN: &str = "a transaction begun with `begin()` is open on this connection; \
     run the statements through it rather than in a batch or another transaction";

impl DoConnection {
    /// Runs `callback` in a transaction: it commits if the callback returns
    /// `Ok`, and rolls back every write the callback made if it returns
    /// `Err`.
    ///
    /// A real interactive transaction, unlike a batch: the callback reads its
    /// own writes, and one statement can use what the one before it returned.
    /// It runs inside `Storage::transaction`, and the callback's value -- or
    /// its error, as it returned it -- is what this resolves to. sqlx's own
    /// `begin()` works too (see [`DoTransactionManager`]); this is the form
    /// that cannot be left open by mistake.
    ///
    /// ```no_run
    /// # async fn create(conn: &sqlx_cloudflare_do::DoConnection, email: String) -> Result<i64, sqlx::Error> {
    /// let id = conn
    ///     .transaction(move |tx| async move {
    ///         let id: i64 = sqlx::query_scalar("INSERT INTO users (email) VALUES (?) RETURNING id")
    ///             .bind(email)
    ///             .fetch_one(&tx)
    ///             .await?;
    ///         sqlx::query("INSERT INTO posts (user_id, title) VALUES (?, 'hello')")
    ///             .bind(id)
    ///             .execute(&tx)
    ///             .await?;
    ///         Ok::<_, sqlx::Error>(id)
    ///     })
    ///     .await?;
    /// # Ok(id)
    /// # }
    /// ```
    ///
    /// # The callback owns what it uses
    ///
    /// `Storage::transaction` takes a `'static` callback, so it cannot borrow
    /// the connection or anything else from the caller: move what it needs
    /// in, and run its queries through the [`DoTransaction`] it is handed.
    ///
    /// # Which `transaction` you get
    ///
    /// sqlx's `Connection` trait has a `transaction` method of its own, taking
    /// `&mut self` and a boxed callback over `&mut Transaction`. With that
    /// trait in scope, `conn.transaction(…)` on a `&mut DoConnection` resolves
    /// to sqlx's -- which works too, through `begin()` and `commit()`. Call
    /// this one on a `&DoConnection`, or as `DoConnection::transaction(&conn,
    /// …)`.
    ///
    /// # It holds the whole object
    ///
    /// The Durable Object delivers no other request until the transaction
    /// ends -- even while the callback awaits something that is not storage.
    /// That is what keeps other requests' writes out of it, and why it should
    /// be short: an awaited `fetch()` inside it stalls every caller of the
    /// object, and a request to the object itself would never be answered.
    ///
    /// # Errors
    ///
    /// The callback's own error, after rolling back; or, converted into `E`,
    /// an error from storage when the transaction could not commit, or when a
    /// transaction begun with `begin()` is still open on this connection.
    pub fn transaction<F, Fut, T, E>(
        &self,
        callback: F,
    ) -> impl Future<Output = Result<T, E>> + Send + '_
    where
        F: FnOnce(DoTransaction) -> Fut + 'static,
        Fut: Future<Output = Result<T, E>> + 'static,
        T: 'static,
        E: From<Error> + 'static,
    {
        let tx = DoTransaction {
            sql: self.sql.clone(),
        };

        // `Storage::transaction` and the callback's future hold JavaScript
        // handles, so neither is `Send`; `SendFuture` asserts it, soundly,
        // because a Durable Object runs on one thread.
        SendFuture::new(async move {
            self.settle().await;

            // Storage would nest this inside the parked transaction, whose
            // callback waits on a commit that cannot come while this runs.
            if self.is_open() {
                return Err(E::from(unsupported(ALREADY_OPEN)));
            }

            // The callback reports through this rather than its return
            // value, which `worker` flattens to a string: the caller gets its
            // value, or its error as it returned it.
            let outcome = Rc::new(RefCell::new(None));
            let slot = Rc::clone(&outcome);

            let committed = self
                .storage
                .transaction(move |_| async move {
                    let result = callback(tx).await;
                    let failed = result.is_err();
                    *slot.borrow_mut() = Some(result);

                    if failed {
                        // Any error will do: rejecting is what rolls back.
                        Err(worker::Error::RustError("the callback failed".into()))
                    } else {
                        Ok(())
                    }
                })
                .await;

            let outcome = outcome.borrow_mut().take();
            match (committed, outcome) {
                (_, Some(Err(error))) => Err(error),
                (Ok(()), Some(Ok(value))) => Ok(value),
                (Err(error), _) => Err(E::from(Error::Protocol(format!(
                    "the Durable Object storage transaction failed: {error}"
                )))),
                (Ok(()), None) => Err(E::from(Error::Protocol(
                    "the Durable Object storage transaction never ran its callback".into(),
                ))),
            }
        })
    }

    /// Whether a transaction `begin()` opened is still open.
    pub(crate) fn is_open(&self) -> bool {
        matches!(*self.open.borrow(), Open::Parked { .. })
    }

    /// Records a `begin()` refused while a transaction is open, so the
    /// `start_rollback` its guard's drop brings is not taken for this one's.
    fn refuse_nested(&mut self, error: Error) -> Error {
        if let Open::Parked { refused, .. } = self.open.get_mut() {
            *refused += 1;
        }
        error
    }

    /// Waits for a dropped transaction's rollback to land.
    ///
    /// sqlx rolls back a dropped `Transaction` synchronously -- it can only
    /// signal the parked transaction, not wait for it -- and a statement run
    /// before the rollback lands would be rolled back with it. So every query
    /// on the connection waits here first.
    pub(crate) async fn settle(&self) {
        let rolling_back = match &*self.open.borrow() {
            Open::RollingBack(done) => Some(done.clone()),
            _ => None,
        };

        if let Some(done) = rolling_back {
            let _ = done.await;
            let mut open = self.open.borrow_mut();
            if matches!(*open, Open::RollingBack(_)) {
                *open = Open::None;
            }
        }
    }
}

/// An open transaction: the executor [`DoConnection::transaction`] hands its
/// callback.
///
/// `&DoTransaction` and `&mut DoTransaction` are sqlx executors, like a
/// connection's references, and every query run through one is part of the
/// transaction.
pub struct DoTransaction {
    pub(crate) sql: worker::SqlStorage,
}

impl DoTransaction {
    // The executor waits on this for a connection; a callback's transaction
    // has no dropped `begin()` to wait for.
    #[allow(
        clippy::unused_async,
        reason = "the executor awaits it alike for both kinds of executor"
    )]
    pub(crate) async fn settle(&self) {}
}

impl Debug for DoTransaction {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("DoTransaction").finish_non_exhaustive()
    }
}

/// How a parked transaction ended, as `Storage::transaction` reported it.
type Ended = Result<(), String>;

/// Where a transaction `begin()` opened stands.
#[derive(Default)]
pub(crate) enum Open {
    #[default]
    None,
    /// Open: `Storage::transaction` is running a callback that waits for
    /// `signal` -- `true` to commit, `false` to roll back -- and `ended`
    /// reports how the transaction finished.
    Parked {
        signal: oneshot::Sender<bool>,
        ended: oneshot::Receiver<Ended>,
        /// `begin()` calls refused while this was open, whose guards have
        /// yet to drop. sqlx builds a `Transaction` before calling `begin`,
        /// and drops it -- calling `start_rollback` -- when `begin` fails; each
        /// such drop is owed to a refusal, not to this transaction.
        refused: usize,
    },
    /// Dropped without a commit: rollback signalled, not yet landed. Shared,
    /// because every query on the connection waits for it.
    RollingBack(Shared<oneshot::Receiver<Ended>>),
}

/// Opens `Storage::transaction` in a task of its own, with a callback that
/// waits for a signal rather than running any statements.
///
/// The statements are the caller's, run on the connection while the callback
/// waits. They are part of the transaction all the same -- Durable Object
/// storage has one database, and the transaction is on it, not on the
/// callback -- and the object delivers no other request until it ends. Both
/// were measured before this was written: the request that opened it can
/// await timers and sub-requests meanwhile, and another request's write, held
/// off until the end, survives the rollback.
///
/// Returns the signal, a receiver that resolves once the callback runs, and
/// one that says how the transaction ended.
fn park(
    storage: Rc<worker::Storage>,
) -> (
    oneshot::Sender<bool>,
    oneshot::Receiver<()>,
    oneshot::Receiver<Ended>,
) {
    let (signal, signalled) = oneshot::channel::<bool>();
    let (started_tx, started) = oneshot::channel();
    let (ended_tx, ended) = oneshot::channel();

    worker::wasm_bindgen_futures::spawn_local(async move {
        let result = storage
            .transaction(move |_| async move {
                let _ = started_tx.send(());
                match signalled.await {
                    Ok(true) => Ok(()),
                    // `false` -- or the connection dropped mid-transaction,
                    // taking the signal with it: roll back either way.
                    _ => Err(worker::Error::RustError(ROLLED_BACK.into())),
                }
            })
            .await;

        let _ = ended_tx.send(result.map_err(|error| error.to_string()));
    });

    (signal, started, ended)
}

/// sqlx's `begin()`, `commit()` and `rollback()` for Durable Object storage.
///
/// `sql.exec` rejects `BEGIN` and `SAVEPOINT`, and `Storage::transaction`
/// wraps a callback -- so `begin()` parks one open: the callback only waits
/// for `commit()` or `rollback()`, while the statements run on the
/// connection, through `&mut *tx`, as with any sqlx driver. They read their
/// own writes, and a rollback undoes them.
///
/// ```no_run
/// # async fn transfer(conn: &mut sqlx_cloudflare_do::DoConnection) -> Result<(), sqlx::Error> {
/// use sqlx::Connection;
///
/// let mut tx = conn.begin().await?;
/// sqlx::query("UPDATE accounts SET balance = balance - 10 WHERE id = 1")
///     .execute(&mut *tx)
///     .await?;
/// sqlx::query("UPDATE accounts SET balance = balance + 10 WHERE id = 2")
///     .execute(&mut *tx)
///     .await?;
/// tx.commit().await?; // dropping `tx` instead rolls both back
/// # Ok(())
/// # }
/// ```
///
/// # It holds the whole object
///
/// While a transaction is open the Durable Object delivers no other request,
/// which keeps other requests' writes out of it -- and means one left open
/// stalls every caller of the object until it is committed, rolled back or
/// dropped. Keep it short, and never await a request to the object itself
/// inside one. [`DoConnection::transaction`] is the form that cannot be left
/// open.
///
/// # Not supported
///
/// Nested `begin()` -- savepoints, to sqlx -- and a custom `BEGIN` statement
/// both fail; so do [`DoConnection::execute_batch`],
/// [`DoConnection::fetch_batch`] and [`DoConnection::transaction`] while a
/// transaction is open, since storage would nest them inside it.
#[derive(Debug)]
pub struct DoTransactionManager;

impl TransactionManager for DoTransactionManager {
    type Database = Do;

    async fn begin(conn: &mut DoConnection, statement: Option<SqlStr>) -> Result<(), Error> {
        conn.settle().await;

        if conn.is_open() {
            return Err(conn.refuse_nested(unsupported(
                "nested transactions (savepoints) are not supported",
            )));
        }

        if statement.is_some() {
            return Err(unsupported(
                "a custom BEGIN statement cannot be run: `sql.exec` rejects them",
            ));
        }

        let (signal, started, mut ended) = park(Rc::clone(&conn.storage));

        // The callback runs once storage grants the transaction; if
        // storage refuses it instead, `ended` says why.
        let refused = match future::select(started, &mut ended).await {
            Either::Left((Ok(()), _)) => None,
            Either::Left((Err(oneshot::Canceled), _)) => Some((&mut ended).await),
            Either::Right((result, _)) => Some(result),
        };

        if let Some(result) = refused {
            let why = match result {
                Ok(Err(why)) => why,
                Ok(Ok(())) => "it ended before it began".to_owned(),
                Err(oneshot::Canceled) => "it never reported back".to_owned(),
            };
            return Err(Error::Protocol(format!(
                "could not begin a Durable Object storage transaction: {why}"
            )));
        }

        *conn.open.get_mut() = Open::Parked {
            signal,
            ended,
            refused: 0,
        };
        Ok(())
    }

    async fn commit(conn: &mut DoConnection) -> Result<(), Error> {
        let Open::Parked { signal, ended, .. } = mem::take(conn.open.get_mut()) else {
            return Err(unsupported("there is no open transaction to commit"));
        };

        let _ = signal.send(true);
        match ended.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(why)) => Err(Error::Protocol(format!(
                "the Durable Object storage transaction did not commit: {why}"
            ))),
            Err(oneshot::Canceled) => Err(Error::Protocol(
                "the Durable Object storage transaction ended without reporting back".into(),
            )),
        }
    }

    async fn rollback(conn: &mut DoConnection) -> Result<(), Error> {
        match mem::take(conn.open.get_mut()) {
            // The callback rejects to roll back, so the transaction
            // "failing" is the rollback succeeding.
            Open::Parked { signal, ended, .. } => {
                let _ = signal.send(false);
                let _ = ended.await;
                Ok(())
            }
            Open::RollingBack(done) => {
                let _ = done.await;
                Ok(())
            }
            Open::None => Ok(()),
        }
    }

    // `Transaction`'s `Drop`: signal the rollback, which cannot be awaited
    // here, and leave the connection to wait for it before its next query --
    // unless the drop is a refused `begin()`'s guard, which owns nothing.
    fn start_rollback(conn: &mut DoConnection) {
        if let Open::Parked { refused, .. } = conn.open.get_mut() {
            if *refused > 0 {
                *refused -= 1;
                return;
            }
        }

        if let Open::Parked { signal, ended, .. } = mem::take(conn.open.get_mut()) {
            let _ = signal.send(false);
            *conn.open.get_mut() = Open::RollingBack(ended.shared());
        }
    }

    fn get_transaction_depth(conn: &DoConnection) -> usize {
        usize::from(conn.is_open())
    }
}
