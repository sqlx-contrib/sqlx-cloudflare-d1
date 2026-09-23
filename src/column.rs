use sqlx_core::column::Column;

use crate::{D1TypeInfo, D1};

/// A column of a D1 result.
#[derive(Debug, Clone)]
pub struct D1Column {
    pub(crate) name: String,
    pub(crate) ordinal: usize,
    /// The type of the column's first non-NULL value, or `Null` if it has
    /// none -- D1 reports no declared types (see [`D1TypeInfo`]).
    pub(crate) type_info: D1TypeInfo,
}

impl Column for D1Column {
    type Database = D1;

    fn ordinal(&self) -> usize {
        self.ordinal
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn type_info(&self) -> &D1TypeInfo {
        &self.type_info
    }
}
