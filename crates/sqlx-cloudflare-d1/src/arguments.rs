use sqlx_cloudflare_core::add_argument;
use sqlx_core::arguments::Arguments;
use sqlx_core::encode::Encode;
use sqlx_core::error::BoxDynError;
use sqlx_core::types::Type;

use crate::{D1ArgumentValue, D1};

/// The parameters bound to one query, in placeholder order.
///
/// `format_placeholder` keeps its default, `?`, which is what D1 expects.
#[derive(Debug, Default, Clone)]
pub struct D1Arguments {
    pub(crate) values: Vec<D1ArgumentValue>,
}

impl D1Arguments {
    pub(crate) fn values(&self) -> &[D1ArgumentValue] {
        &self.values
    }
}

impl Arguments for D1Arguments {
    type Database = D1;

    fn reserve(&mut self, additional: usize, _size: usize) {
        self.values.reserve(additional);
    }

    fn add<'t, T>(&mut self, value: T) -> Result<(), BoxDynError>
    where
        T: Encode<'t, D1> + Type<D1>,
    {
        add_argument(&mut self.values, value)
    }

    fn len(&self) -> usize {
        self.values.len()
    }
}

#[cfg(test)]
mod tests {
    use sqlx_core::arguments::Arguments;

    use super::{D1ArgumentValue, D1Arguments};

    #[test]
    fn values_line_up_with_placeholders() {
        let mut arguments = D1Arguments::default();

        arguments.add(7_i64).unwrap();
        arguments.add(Option::<String>::None).unwrap();
        arguments.add("x").unwrap();
        arguments.add(true).unwrap();
        arguments.add(1.5_f64).unwrap();
        arguments.add(vec![1_u8, 2]).unwrap();

        assert_eq!(
            arguments.values(),
            [
                D1ArgumentValue::Integer(7),
                D1ArgumentValue::Null,
                D1ArgumentValue::Text("x".into()),
                D1ArgumentValue::Integer(1),
                D1ArgumentValue::Real(1.5),
                D1ArgumentValue::Blob(vec![1, 2]),
            ]
        );
    }

    #[test]
    fn an_integer_beyond_2_53_is_refused_and_leaves_no_trace() {
        let mut arguments = D1Arguments::default();

        arguments.add(1_i64).unwrap();
        assert!(arguments.add(1_i64 << 53).is_err());
        assert!(arguments.add(i64::MIN).is_err());
        arguments.add(2_i64).unwrap();

        // The failed binds pushed nothing, so `?2` is still the second value.
        assert_eq!(
            arguments.values(),
            [D1ArgumentValue::Integer(1), D1ArgumentValue::Integer(2)]
        );
    }
}
