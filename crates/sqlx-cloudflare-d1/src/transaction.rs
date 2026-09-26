use std::future::{self, Future};

use sqlx_core::error::Error;
use sqlx_core::sql_str::SqlStr;
use sqlx_core::transaction::TransactionManager;

use crate::error::unsupported;
use crate::{D1Connection, D1};

const NO_TRANSACTIONS: &str = "D1 has no interactive transactions; \
     send the statements together with `D1Connection::batch`, which D1 runs atomically";

/// Refuses every transaction.
///
/// D1 does not hold a transaction open across calls, so `BEGIN` cannot be
/// followed by anything but its own statement. Buffering writes until
/// `commit` would look like a transaction and not be one -- reads inside it
/// would see the database as it was before -- which is worse than an error.
/// So `conn.begin()` compiles and fails at runtime, and the atomic
/// multi-statement case goes through `D1Connection::batch`.
#[derive(Debug)]
pub struct D1TransactionManager;

impl TransactionManager for D1TransactionManager {
    type Database = D1;

    fn begin(
        _conn: &mut D1Connection,
        _statement: Option<SqlStr>,
    ) -> impl Future<Output = Result<(), Error>> + Send + '_ {
        future::ready(Err(unsupported(NO_TRANSACTIONS)))
    }

    // Unreachable in practice: `begin` never succeeds, so there is never a
    // transaction to end. They still answer rather than panic.
    fn commit(_conn: &mut D1Connection) -> impl Future<Output = Result<(), Error>> + Send + '_ {
        future::ready(Err(unsupported(NO_TRANSACTIONS)))
    }

    fn rollback(_conn: &mut D1Connection) -> impl Future<Output = Result<(), Error>> + Send + '_ {
        future::ready(Err(unsupported(NO_TRANSACTIONS)))
    }

    // `Transaction` calls this from `Drop` when `begin` failed -- it cannot
    // tell a failed begin from an abandoned transaction -- so it must be a
    // quiet no-op, not an error.
    fn start_rollback(_conn: &mut D1Connection) {}

    fn get_transaction_depth(_conn: &D1Connection) -> usize {
        0
    }
}
