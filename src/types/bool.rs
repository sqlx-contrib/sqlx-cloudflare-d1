use sqlx_core::decode::Decode;
use sqlx_core::encode::{Encode, IsNull};
use sqlx_core::error::BoxDynError;
use sqlx_core::types::Type;

use crate::{D1ArgumentValue, D1TypeInfo, D1ValueRef, D1};

// SQLite has no boolean storage class; `true` is stored as 1. A JavaScript
// `boolean` coming back from D1 is converted to 0/1 before it gets here.
impl Type<D1> for bool {
    fn type_info() -> D1TypeInfo {
        D1TypeInfo::Integer
    }
}

impl Encode<'_, D1> for bool {
    fn encode_by_ref(&self, buf: &mut Vec<D1ArgumentValue>) -> Result<IsNull, BoxDynError> {
        buf.push(D1ArgumentValue::Integer(i64::from(*self)));
        Ok(IsNull::No)
    }
}

impl<'r> Decode<'r, D1> for bool {
    // Any non-zero integer, as SQLite and sqlx-sqlite read it.
    fn decode(value: D1ValueRef<'r>) -> Result<Self, BoxDynError> {
        Ok(value.int()? != 0)
    }
}
