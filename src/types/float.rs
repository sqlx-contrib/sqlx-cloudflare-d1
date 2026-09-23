use sqlx_core::decode::Decode;
use sqlx_core::encode::{Encode, IsNull};
use sqlx_core::error::BoxDynError;
use sqlx_core::types::Type;

use crate::{D1ArgumentValue, D1TypeInfo, D1ValueRef, D1};

impl Type<D1> for f64 {
    fn type_info() -> D1TypeInfo {
        D1TypeInfo::Real
    }

    // `INTEGER` too: a whole number stored as REAL comes back typed INTEGER,
    // because JavaScript cannot tell `3.0` from `3`.
    fn compatible(ty: &D1TypeInfo) -> bool {
        matches!(ty, D1TypeInfo::Real | D1TypeInfo::Integer)
    }
}

impl Encode<'_, D1> for f64 {
    fn encode_by_ref(&self, buf: &mut Vec<D1ArgumentValue>) -> Result<IsNull, BoxDynError> {
        buf.push(D1ArgumentValue::Real(*self));
        Ok(IsNull::No)
    }
}

impl<'r> Decode<'r, D1> for f64 {
    fn decode(value: D1ValueRef<'r>) -> Result<Self, BoxDynError> {
        value.real()
    }
}

impl Type<D1> for f32 {
    fn type_info() -> D1TypeInfo {
        D1TypeInfo::Real
    }

    fn compatible(ty: &D1TypeInfo) -> bool {
        <f64 as Type<D1>>::compatible(ty)
    }
}

impl Encode<'_, D1> for f32 {
    fn encode_by_ref(&self, buf: &mut Vec<D1ArgumentValue>) -> Result<IsNull, BoxDynError> {
        buf.push(D1ArgumentValue::Real(f64::from(*self)));
        Ok(IsNull::No)
    }
}

impl<'r> Decode<'r, D1> for f32 {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "narrowing a stored REAL to f32 is what asking for an f32 means; sqlx-sqlite does the same"
    )]
    fn decode(value: D1ValueRef<'r>) -> Result<Self, BoxDynError> {
        Ok(value.real()? as f32)
    }
}
