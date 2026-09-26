use sqlx_core::database::Database;
use sqlx_core::encode::{Encode, IsNull};
use sqlx_core::error::BoxDynError;
use sqlx_core::types::Type;

/// One bound parameter, in the shape these backends accept across the
/// JavaScript boundary.
///
/// Owned rather than borrowed: sqlx 0.9's `Arguments` has no lifetime, and the
/// conversion to a JavaScript value happens once, when the query runs.
///
/// Non-exhaustive so that a backend growing a representation -- a `BigInt`,
/// say -- is not a breaking change for `Encode` impls outside this crate.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ArgumentValue {
    /// SQL `NULL`: JavaScript `null`.
    Null,
    /// Within ±(2^53 − 1), because the backend takes a JavaScript number and
    /// not a `BigInt` -- anything wider would be rounded on the way in.
    Integer(i64),
    /// A JavaScript number, stored as `REAL`.
    Real(f64),
    /// A JavaScript string, stored as `TEXT`.
    Text(String),
    /// An `ArrayBuffer`, stored as `BLOB`.
    Blob(Vec<u8>),
}

/// Encodes `value` onto `values`, as one placeholder's worth.
///
/// What a driver's `Arguments::add` does: whatever the encoder does, exactly
/// one value lands for the placeholder, or none if it fails.
///
/// # Errors
///
/// When encoding fails, in which case `values` is left as it was.
pub fn add_argument<'t, DB, T>(values: &mut Vec<ArgumentValue>, value: T) -> Result<(), BoxDynError>
where
    DB: Database<ArgumentBuffer = Vec<ArgumentValue>>,
    T: Encode<'t, DB> + Type<DB>,
{
    let len = values.len();

    match value.encode(values) {
        Ok(IsNull::Yes) => {
            // An encoder that reports NULL may still have pushed something;
            // the placeholder must line up with exactly one value.
            values.truncate(len);
            values.push(ArgumentValue::Null);
        }
        Ok(IsNull::No) => {}
        Err(error) => {
            // Leave the arguments as they were, so a failed bind cannot shift
            // every later parameter one placeholder to the right.
            values.truncate(len);
            return Err(error);
        }
    }

    Ok(())
}
