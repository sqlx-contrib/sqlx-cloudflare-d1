use std::borrow::Cow;
use std::fmt::{self, Display, Formatter};

use sqlx_core::error::BoxDynError;
use sqlx_core::type_info::TypeInfo;
use sqlx_core::value::{Value, ValueRef};

use crate::D1;

/// The storage class of a single value, which is all D1 reports.
///
/// There is no declared column type to go on: D1 hands back JavaScript values,
/// not SQLite's column metadata. So the type is read off each value as it
/// arrives, and a column's type is the type of whatever is in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum D1TypeInfo {
    Null,
    Integer,
    Real,
    Text,
    Blob,
}

impl Display for D1TypeInfo {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.pad(self.name())
    }
}

impl TypeInfo for D1TypeInfo {
    fn is_null(&self) -> bool {
        matches!(self, D1TypeInfo::Null)
    }

    fn name(&self) -> &str {
        match self {
            D1TypeInfo::Null => "NULL",
            D1TypeInfo::Integer => "INTEGER",
            D1TypeInfo::Real => "REAL",
            D1TypeInfo::Text => "TEXT",
            D1TypeInfo::Blob => "BLOB",
        }
    }
}

/// A value read out of a D1 result, already converted from JavaScript.
///
/// Owned, because D1 returns the whole result set as JavaScript values and
/// there is nothing on the Rust side for a row to borrow from.
#[derive(Debug, Clone, PartialEq)]
pub struct D1Value(pub(crate) D1ValueData);

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum D1ValueData {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// The largest integer a JavaScript number holds exactly: 2^53 − 1.
/// `Number.MAX_SAFE_INTEGER`, in other words.
pub(crate) const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

/// `value`, if a JavaScript number can carry it without rounding.
///
/// D1 takes and returns JavaScript numbers, never `BigInt`, so an integer
/// outside ±(2^53 − 1) would come back as a different integer. That is an
/// error here rather than a silently wrong row.
pub(crate) fn safe_integer(value: i64) -> Result<i64, BoxDynError> {
    if (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "{value} is outside ±(2^53 − 1), the integers D1 can store without rounding: \
             it passes them as JavaScript numbers"
        )
        .into())
    }
}

impl D1ValueData {
    /// A JavaScript number, as the storage class it most likely came from.
    ///
    /// JavaScript has one number type, so `3` and `3.0` arrive the same. A
    /// whole number in the safe range is taken as an `Integer` -- which is
    /// why `f64` decodes from `Integer` as well as `Real`. Anything else,
    /// including whole numbers too large to be exact, is a `Real`.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "the range check makes both casts exact"
    )]
    pub(crate) fn from_number(value: f64) -> Self {
        if value.fract() == 0.0 && value.abs() <= MAX_SAFE_INTEGER as f64 {
            D1ValueData::Integer(value as i64)
        } else {
            D1ValueData::Real(value)
        }
    }

    pub(crate) fn type_info(&self) -> D1TypeInfo {
        match self {
            D1ValueData::Null => D1TypeInfo::Null,
            D1ValueData::Integer(_) => D1TypeInfo::Integer,
            D1ValueData::Real(_) => D1TypeInfo::Real,
            D1ValueData::Text(_) => D1TypeInfo::Text,
            D1ValueData::Blob(_) => D1TypeInfo::Blob,
        }
    }
}

impl Value for D1Value {
    type Database = D1;

    fn as_ref(&self) -> D1ValueRef<'_> {
        D1ValueRef(self)
    }

    fn type_info(&self) -> Cow<'_, D1TypeInfo> {
        Cow::Owned(self.0.type_info())
    }

    fn is_null(&self) -> bool {
        matches!(self.0, D1ValueData::Null)
    }
}

/// A borrowed [`D1Value`], which is what `Decode` implementations read.
#[derive(Debug, Clone, Copy)]
pub struct D1ValueRef<'r>(pub(crate) &'r D1Value);

// The accessors `Decode` implementations use. Each accepts exactly the
// storage classes the matching `Type::compatible` does, so `try_get` and
// `try_get_unchecked` agree about what decodes.
impl<'r> D1ValueRef<'r> {
    pub(crate) fn int(self) -> Result<i64, BoxDynError> {
        match self.0 .0 {
            D1ValueData::Integer(value) => Ok(value),
            ref other => Err(mismatch("an INTEGER", other)),
        }
    }

    #[allow(
        clippy::cast_precision_loss,
        reason = "an INTEGER from D1 is within ±(2^53 − 1), so it converts exactly"
    )]
    pub(crate) fn real(self) -> Result<f64, BoxDynError> {
        match self.0 .0 {
            D1ValueData::Real(value) => Ok(value),
            D1ValueData::Integer(value) => Ok(value as f64),
            ref other => Err(mismatch("a REAL", other)),
        }
    }

    pub(crate) fn text(self) -> Result<&'r str, BoxDynError> {
        match &self.0 .0 {
            D1ValueData::Text(value) => Ok(value),
            other => Err(mismatch("TEXT", other)),
        }
    }

    pub(crate) fn blob(self) -> Result<&'r [u8], BoxDynError> {
        match &self.0 .0 {
            D1ValueData::Blob(value) => Ok(value),
            // As sqlx-sqlite does: SQLite hands TEXT to `sqlite3_column_blob`
            // as its bytes.
            D1ValueData::Text(value) => Ok(value.as_bytes()),
            other => Err(mismatch("a BLOB", other)),
        }
    }
}

fn mismatch(expected: &str, found: &D1ValueData) -> BoxDynError {
    format!("expected {expected}, found {}", found.type_info()).into()
}

impl ValueRef<'_> for D1ValueRef<'_> {
    type Database = D1;

    fn to_owned(&self) -> D1Value {
        self.0.clone()
    }

    fn type_info(&self) -> Cow<'_, D1TypeInfo> {
        Cow::Owned(self.0 .0.type_info())
    }

    fn is_null(&self) -> bool {
        matches!(self.0 .0, D1ValueData::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::{safe_integer, D1ValueData, MAX_SAFE_INTEGER};

    #[test]
    fn whole_numbers_in_the_safe_range_are_integers() {
        assert_eq!(D1ValueData::from_number(3.0), D1ValueData::Integer(3));
        assert_eq!(D1ValueData::from_number(-0.0), D1ValueData::Integer(0));
        assert_eq!(
            D1ValueData::from_number(9_007_199_254_740_991.0),
            D1ValueData::Integer(MAX_SAFE_INTEGER)
        );
    }

    #[test]
    fn everything_else_is_real() {
        assert_eq!(D1ValueData::from_number(1.5), D1ValueData::Real(1.5));
        // 2^53: whole, but no longer exact -- 2^53 + 1 would read the same.
        assert_eq!(
            D1ValueData::from_number(9_007_199_254_740_992.0),
            D1ValueData::Real(9_007_199_254_740_992.0)
        );
        assert!(matches!(
            D1ValueData::from_number(f64::INFINITY),
            D1ValueData::Real(_)
        ));
        assert!(matches!(
            D1ValueData::from_number(f64::NAN),
            D1ValueData::Real(_)
        ));
    }

    #[test]
    fn safe_integer_accepts_exactly_the_safe_range() {
        assert!(safe_integer(MAX_SAFE_INTEGER).is_ok());
        assert!(safe_integer(-MAX_SAFE_INTEGER).is_ok());
        assert!(safe_integer(MAX_SAFE_INTEGER + 1).is_err());
        assert!(safe_integer(-MAX_SAFE_INTEGER - 1).is_err());
        assert!(safe_integer(i64::MIN).is_err());
    }
}
