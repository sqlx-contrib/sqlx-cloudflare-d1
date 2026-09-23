use sqlx_core::sql_str::SqlStr;
use sqlx_core::statement::Statement;
use sqlx_core::Either;

use crate::{D1Arguments, D1Column, D1TypeInfo, D1};

/// A "prepared" D1 statement: the SQL and nothing else.
///
/// D1 has no server-side prepare that reports parameters or columns, so there
/// is nothing to learn by preparing ahead of time. This exists because sqlx's
/// `Executor::prepare` has to return something.
#[derive(Debug, Clone)]
pub struct D1Statement {
    pub(crate) sql: SqlStr,
}

impl Statement for D1Statement {
    type Database = D1;

    fn into_sql(self) -> SqlStr {
        self.sql
    }

    fn sql(&self) -> &SqlStr {
        &self.sql
    }

    fn parameters(&self) -> Option<Either<&[D1TypeInfo], usize>> {
        None
    }

    fn columns(&self) -> &[D1Column] {
        &[]
    }

    sqlx_core::impl_statement_query!(D1Arguments);
}
