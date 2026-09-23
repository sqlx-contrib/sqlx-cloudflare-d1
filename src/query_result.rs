/// What a statement did, from D1's `meta`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct D1QueryResult {
    pub(crate) rows_affected: u64,
    pub(crate) last_insert_rowid: Option<i64>,
}

impl D1QueryResult {
    /// Rows changed by the statement: D1's `meta.changes`.
    #[must_use]
    pub fn rows_affected(&self) -> u64 {
        self.rows_affected
    }

    /// D1's `meta.last_row_id`, when it reports one.
    #[must_use]
    pub fn last_insert_rowid(&self) -> Option<i64> {
        self.last_insert_rowid
    }
}

impl Extend<D1QueryResult> for D1QueryResult {
    fn extend<T: IntoIterator<Item = D1QueryResult>>(&mut self, iter: T) {
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
    use super::D1QueryResult;

    #[test]
    fn extend_sums_changes_and_keeps_the_last_reported_rowid() {
        let mut result = D1QueryResult::default();

        result.extend([
            D1QueryResult {
                rows_affected: 2,
                last_insert_rowid: Some(7),
            },
            D1QueryResult {
                rows_affected: 1,
                last_insert_rowid: Some(9),
            },
            D1QueryResult {
                rows_affected: 0,
                last_insert_rowid: None,
            },
        ]);

        assert_eq!(result.rows_affected(), 3);
        assert_eq!(result.last_insert_rowid(), Some(9));
    }
}
