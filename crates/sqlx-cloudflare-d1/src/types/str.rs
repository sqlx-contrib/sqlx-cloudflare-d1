use std::borrow::Cow;
use std::sync::Arc;

use sqlx_core::decode::Decode;
use sqlx_core::encode::{Encode, IsNull};
use sqlx_core::error::BoxDynError;
use sqlx_core::types::Type;

use crate::{D1ArgumentValue, D1TypeInfo, D1ValueRef, D1};

impl Type<D1> for str {
    fn type_info() -> D1TypeInfo {
        D1TypeInfo::Text
    }
}

impl Encode<'_, D1> for &'_ str {
    fn encode_by_ref(&self, buf: &mut Vec<D1ArgumentValue>) -> Result<IsNull, BoxDynError> {
        buf.push(D1ArgumentValue::Text((*self).to_owned()));
        Ok(IsNull::No)
    }
}

impl<'r> Decode<'r, D1> for &'r str {
    fn decode(value: D1ValueRef<'r>) -> Result<Self, BoxDynError> {
        value.text()
    }
}

impl Type<D1> for String {
    fn type_info() -> D1TypeInfo {
        <str as Type<D1>>::type_info()
    }
}

impl Encode<'_, D1> for String {
    fn encode(self, buf: &mut Vec<D1ArgumentValue>) -> Result<IsNull, BoxDynError> {
        buf.push(D1ArgumentValue::Text(self));
        Ok(IsNull::No)
    }

    fn encode_by_ref(&self, buf: &mut Vec<D1ArgumentValue>) -> Result<IsNull, BoxDynError> {
        buf.push(D1ArgumentValue::Text(self.clone()));
        Ok(IsNull::No)
    }
}

impl<'r> Decode<'r, D1> for String {
    fn decode(value: D1ValueRef<'r>) -> Result<Self, BoxDynError> {
        value.text().map(ToOwned::to_owned)
    }
}

// sqlx-core's smart-pointer impls cover `Box<T>` and friends only for sized
// `T`, so the unsized ones each need their own -- as sqlx-sqlite has.
macro_rules! impl_encode_str_via_ref {
    ($($ty:ty),*) => {$(
        impl Encode<'_, D1> for $ty {
            fn encode_by_ref(&self, buf: &mut Vec<D1ArgumentValue>) -> Result<IsNull, BoxDynError> {
                <&str as Encode<D1>>::encode(&**self, buf)
            }
        }
    )*};
}

impl_encode_str_via_ref!(Box<str>, Arc<str>, Cow<'_, str>);
