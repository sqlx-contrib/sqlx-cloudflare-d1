/// What a statement did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct QueryResult {
    rows_affected: u64,
    last_insert_rowid: Option<i64>,
}

impl QueryResult {
    /// A result reporting `rows_affected` and, when the backend knows one,
    /// `last_insert_rowid`.
    #[must_use]
    pub fn new(rows_affected: u64, last_insert_rowid: Option<i64>) -> Self {
        Self {
            rows_affected,
            last_insert_rowid,
        }
    }

    /// Rows the statement inserted, updated or deleted.
    #[must_use]
    pub fn rows_affected(&self) -> u64 {
        self.rows_affected
    }

    /// The rowid of the last row inserted, when the backend reports one.
    #[must_use]
    pub fn last_insert_rowid(&self) -> Option<i64> {
        self.last_insert_rowid
    }
}

impl Extend<QueryResult> for QueryResult {
    fn extend<T: IntoIterator<Item = QueryResult>>(&mut self, iter: T) {
        for result in iter {
            self.rows_affected += result.rows_affected;
            // The last statement that reported a rowid, not the last
            // statement: a trailing SELECT reporting none should not erase
            // the INSERT before it.
            self.last_insert_rowid = result.last_insert_rowid.or(self.last_insert_rowid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::QueryResult;

    #[test]
    fn extend_sums_changes_and_keeps_the_last_reported_rowid() {
        let mut result = QueryResult::default();

        result.extend([
            QueryResult::new(2, Some(7)),
            QueryResult::new(1, Some(9)),
            QueryResult::new(0, None),
        ]);

        assert_eq!(result.rows_affected(), 3);
        assert_eq!(result.last_insert_rowid(), Some(9));
    }
}
