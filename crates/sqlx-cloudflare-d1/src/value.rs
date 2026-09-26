use std::borrow::Cow;

use sqlx_cloudflare_core::{AsValue, Value as CoreValue};
use sqlx_core::value::{Value, ValueRef};

use crate::{D1TypeInfo, D1};

/// A value read out of a D1 result, already converted from JavaScript.
///
/// Owned, because D1 returns the whole result set as JavaScript values and
/// there is nothing on the Rust side for a row to borrow from.
#[derive(Debug, Clone, PartialEq)]
pub struct D1Value(pub(crate) CoreValue);

impl Value for D1Value {
    type Database = D1;

    fn as_ref(&self) -> D1ValueRef<'_> {
        D1ValueRef(&self.0)
    }

    fn type_info(&self) -> Cow<'_, D1TypeInfo> {
        Cow::Owned(self.0.type_info())
    }

    fn is_null(&self) -> bool {
        self.0.is_null()
    }
}

/// A borrowed [`D1Value`], which is what `Decode` implementations read.
#[derive(Debug, Clone, Copy)]
pub struct D1ValueRef<'r>(pub(crate) &'r CoreValue);

impl<'r> AsValue<'r> for D1ValueRef<'r> {
    fn as_value(&self) -> &'r CoreValue {
        self.0
    }
}

impl ValueRef<'_> for D1ValueRef<'_> {
    type Database = D1;

    fn to_owned(&self) -> D1Value {
        D1Value(self.0.clone())
    }

    fn type_info(&self) -> Cow<'_, D1TypeInfo> {
        Cow::Owned(self.0.type_info())
    }

    fn is_null(&self) -> bool {
        self.0.is_null()
    }
}
