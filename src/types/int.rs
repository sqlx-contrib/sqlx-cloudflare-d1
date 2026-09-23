use sqlx_core::decode::Decode;
use sqlx_core::encode::{Encode, IsNull};
use sqlx_core::error::BoxDynError;
use sqlx_core::types::Type;

use crate::value::safe_integer;
use crate::{D1ArgumentValue, D1TypeInfo, D1ValueRef, D1};

// Every integer narrower than `i64` fits a JavaScript number exactly, so these
// only need the range check on the way out.
macro_rules! impl_narrow_int {
    ($($ty:ty),*) => {$(
        impl Type<D1> for $ty {
            fn type_info() -> D1TypeInfo {
                D1TypeInfo::Integer
            }
        }

        impl Encode<'_, D1> for $ty {
            fn encode_by_ref(&self, buf: &mut Vec<D1ArgumentValue>) -> Result<IsNull, BoxDynError> {
                buf.push(D1ArgumentValue::Integer(i64::from(*self)));
                Ok(IsNull::No)
            }
        }

        impl<'r> Decode<'r, D1> for $ty {
            fn decode(value: D1ValueRef<'r>) -> Result<Self, BoxDynError> {
                Ok(<$ty>::try_from(value.int()?)?)
            }
        }
    )*};
}

impl_narrow_int!(i8, i16, i32, u8, u16, u32);

impl Type<D1> for i64 {
    fn type_info() -> D1TypeInfo {
        D1TypeInfo::Integer
    }
}

impl Encode<'_, D1> for i64 {
    fn encode_by_ref(&self, buf: &mut Vec<D1ArgumentValue>) -> Result<IsNull, BoxDynError> {
        buf.push(D1ArgumentValue::Integer(safe_integer(*self)?));
        Ok(IsNull::No)
    }
}

impl<'r> Decode<'r, D1> for i64 {
    // No range check needed here: an `Integer` value only ever comes from a
    // number already classified as exact (see `D1ValueData::from_number`).
    fn decode(value: D1ValueRef<'r>) -> Result<Self, BoxDynError> {
        value.int()
    }
}
