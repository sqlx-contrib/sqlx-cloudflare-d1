use std::sync::Arc;

use sqlx_core::decode::Decode;
use sqlx_core::encode::{Encode, IsNull};
use sqlx_core::error::BoxDynError;
use sqlx_core::types::Type;

use crate::{D1ArgumentValue, D1TypeInfo, D1ValueRef, D1};

impl Type<D1> for [u8] {
    fn type_info() -> D1TypeInfo {
        D1TypeInfo::Blob
    }

    // `TEXT` as well, read as its bytes -- sqlx-sqlite accepts the same.
    fn compatible(ty: &D1TypeInfo) -> bool {
        matches!(ty, D1TypeInfo::Blob | D1TypeInfo::Text)
    }
}

impl Encode<'_, D1> for &'_ [u8] {
    fn encode_by_ref(&self, buf: &mut Vec<D1ArgumentValue>) -> Result<IsNull, BoxDynError> {
        buf.push(D1ArgumentValue::Blob(self.to_vec()));
        Ok(IsNull::No)
    }
}

impl<'r> Decode<'r, D1> for &'r [u8] {
    fn decode(value: D1ValueRef<'r>) -> Result<Self, BoxDynError> {
        value.blob()
    }
}

impl Type<D1> for Vec<u8> {
    fn type_info() -> D1TypeInfo {
        <[u8] as Type<D1>>::type_info()
    }

    fn compatible(ty: &D1TypeInfo) -> bool {
        <[u8] as Type<D1>>::compatible(ty)
    }
}

impl Encode<'_, D1> for Vec<u8> {
    fn encode(self, buf: &mut Vec<D1ArgumentValue>) -> Result<IsNull, BoxDynError> {
        buf.push(D1ArgumentValue::Blob(self));
        Ok(IsNull::No)
    }

    fn encode_by_ref(&self, buf: &mut Vec<D1ArgumentValue>) -> Result<IsNull, BoxDynError> {
        buf.push(D1ArgumentValue::Blob(self.clone()));
        Ok(IsNull::No)
    }
}

impl<'r> Decode<'r, D1> for Vec<u8> {
    fn decode(value: D1ValueRef<'r>) -> Result<Self, BoxDynError> {
        value.blob().map(<[u8]>::to_vec)
    }
}

// As for `str`: the unsized smart pointers need their own impls.
macro_rules! impl_encode_bytes_via_ref {
    ($($ty:ty),*) => {$(
        impl Encode<'_, D1> for $ty {
            fn encode_by_ref(&self, buf: &mut Vec<D1ArgumentValue>) -> Result<IsNull, BoxDynError> {
                <&[u8] as Encode<D1>>::encode(&**self, buf)
            }
        }
    )*};
}

impl_encode_bytes_via_ref!(Box<[u8]>, Arc<[u8]>);
