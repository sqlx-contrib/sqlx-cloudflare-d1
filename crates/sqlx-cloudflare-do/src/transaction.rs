use std::future::{self, Future};

use sqlx_core::error::Error;
use sqlx_core::sql_str::SqlStr;
use sqlx_core::transaction::TransactionManager;

use crate::error::unsupported;
use crate::{Do, DoConnection};

const NO_TRANSACTIONS: &str = "Durable Object storage has no interactive transactions; \
     send the statements together with `DoConnection::execute_batch`, \
     which runs them atomically";

/// Refuses every transaction.
///
/// `sql.exec` rejects `BEGIN` and `SAVEPOINT` outright: the only transactions
/// Durable Object storage offers wrap a callback, and sqlx's `begin` and
/// `commit` are two separate calls with the caller's code in between.
/// Buffering writes until `commit` would look like a transaction and not be
/// one -- reads inside it would see the database as it was before -- which is
/// worse than an error. So `conn.begin()` compiles and fails at runtime, and
/// the atomic multi-statement case goes through `DoConnection::execute_batch`.
#[derive(Debug)]
pub struct DoTransactionManager;

impl TransactionManager for DoTransactionManager {
    type Database = Do;

    fn begin(
        _conn: &mut DoConnection,
        _statement: Option<SqlStr>,
    ) -> impl Future<Output = Result<(), Error>> + Send + '_ {
        future::ready(Err(unsupported(NO_TRANSACTIONS)))
    }

    // Unreachable in practice: `begin` never succeeds, so there is never a
    // transaction to end. They still answer rather than panic.
    fn commit(_conn: &mut DoConnection) -> impl Future<Output = Result<(), Error>> + Send + '_ {
        future::ready(Err(unsupported(NO_TRANSACTIONS)))
    }

    fn rollback(_conn: &mut DoConnection) -> impl Future<Output = Result<(), Error>> + Send + '_ {
        future::ready(Err(unsupported(NO_TRANSACTIONS)))
    }

    // `Transaction` calls this from `Drop` when `begin` failed -- it cannot
    // tell a failed begin from an abandoned transaction -- so it must be a
    // quiet no-op, not an error.
    fn start_rollback(_conn: &mut DoConnection) {}

    fn get_transaction_depth(_conn: &DoConnection) -> usize {
        0
    }
}
