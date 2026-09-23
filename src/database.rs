use sqlx_core::database::Database;

use crate::{
    D1ArgumentValue, D1Arguments, D1Column, D1Connection, D1QueryResult, D1Row, D1Statement,
    D1TransactionManager, D1TypeInfo, D1Value, D1ValueRef,
};

/// The Cloudflare D1 database driver: the type sqlx code names in
/// `Executor<'_, Database = D1>`, `sqlx::query_as::<D1, _>` and so on.
#[derive(Debug)]
pub struct D1;

impl Database for D1 {
    type Connection = D1Connection;

    type TransactionManager = D1TransactionManager;

    type Row = D1Row;

    type QueryResult = D1QueryResult;

    type Column = D1Column;

    type TypeInfo = D1TypeInfo;

    type Value = D1Value;
    type ValueRef<'r> = D1ValueRef<'r>;

    type Arguments = D1Arguments;
    type ArgumentBuffer = Vec<D1ArgumentValue>;

    type Statement = D1Statement;

    const NAME: &'static str = "D1";

    const URL_SCHEMES: &'static [&'static str] = &["d1"];
}
