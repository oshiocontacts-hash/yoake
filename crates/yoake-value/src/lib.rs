#![forbid(unsafe_code)]

pub mod wire;

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Unit,
    Boolean(bool),
    Signed(i64),
    Unsigned(u64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Record(BTreeMap<String, Value>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueType {
    Unit,
    Bool,
    S8,
    U8,
    S16,
    U16,
    S32,
    U32,
    S64,
    U64,
    F32,
    F64,
    String,
    Bytes,
}

impl ValueType {
    pub fn accepts(self, value: &Value) -> bool {
        match (self, value) {
            (Self::Unit, Value::Unit)
            | (Self::Bool, Value::Boolean(_))
            | (Self::S64, Value::Signed(_))
            | (Self::U64, Value::Unsigned(_))
            | (Self::F64, Value::Float(_))
            | (Self::String, Value::Text(_))
            | (Self::Bytes, Value::Bytes(_)) => true,
            (Self::S8, Value::Signed(value)) => i8::try_from(*value).is_ok(),
            (Self::S16, Value::Signed(value)) => i16::try_from(*value).is_ok(),
            (Self::S32, Value::Signed(value)) => i32::try_from(*value).is_ok(),
            (Self::U8, Value::Unsigned(value)) => u8::try_from(*value).is_ok(),
            (Self::U16, Value::Unsigned(value)) => u16::try_from(*value).is_ok(),
            (Self::U32, Value::Unsigned(value)) => u32::try_from(*value).is_ok(),
            (Self::F32, Value::Float(value)) => {
                !value.is_finite() || value.abs() <= f64::from(f32::MAX)
            }
            _ => false,
        }
    }
}

pub trait InjectValue: Sized {
    const TYPE: ValueType;

    fn into_inject_value(self) -> Value;
    fn from_inject_value(value: Value) -> Option<Self>;
}

impl InjectValue for () {
    const TYPE: ValueType = ValueType::Unit;

    fn into_inject_value(self) -> Value {
        Value::Unit
    }

    fn from_inject_value(value: Value) -> Option<Self> {
        matches!(value, Value::Unit).then_some(())
    }
}

impl InjectValue for bool {
    const TYPE: ValueType = ValueType::Bool;

    fn into_inject_value(self) -> Value {
        Value::Boolean(self)
    }

    fn from_inject_value(value: Value) -> Option<Self> {
        match value {
            Value::Boolean(value) => Some(value),
            _ => None,
        }
    }
}

impl InjectValue for String {
    const TYPE: ValueType = ValueType::String;

    fn into_inject_value(self) -> Value {
        Value::Text(self)
    }

    fn from_inject_value(value: Value) -> Option<Self> {
        match value {
            Value::Text(value) => Some(value),
            _ => None,
        }
    }
}

impl InjectValue for Vec<u8> {
    const TYPE: ValueType = ValueType::Bytes;

    fn into_inject_value(self) -> Value {
        Value::Bytes(self)
    }

    fn from_inject_value(value: Value) -> Option<Self> {
        match value {
            Value::Bytes(value) => Some(value),
            _ => None,
        }
    }
}

macro_rules! integer_values {
    ($variant:ident; $($native:ty => $kind:ident),* $(,)?) => {
        $(
            impl InjectValue for $native {
                const TYPE: ValueType = ValueType::$kind;

                fn into_inject_value(self) -> Value {
                    Value::$variant(self.into())
                }

                fn from_inject_value(value: Value) -> Option<Self> {
                    match value {
                        Value::$variant(value) => Self::try_from(value).ok(),
                        _ => None,
                    }
                }
            }
        )*
    };
}

integer_values!(Signed; i8 => S8, i16 => S16, i32 => S32, i64 => S64);
integer_values!(Unsigned; u8 => U8, u16 => U16, u32 => U32, u64 => U64);

impl InjectValue for f32 {
    const TYPE: ValueType = ValueType::F32;

    fn into_inject_value(self) -> Value {
        Value::Float(f64::from(self))
    }

    fn from_inject_value(value: Value) -> Option<Self> {
        if !Self::TYPE.accepts(&value) {
            return None;
        }
        match value {
            Value::Float(value) => Some(value as Self),
            _ => None,
        }
    }
}

impl InjectValue for f64 {
    const TYPE: ValueType = ValueType::F64;

    fn into_inject_value(self) -> Value {
        Value::Float(self)
    }

    fn from_inject_value(value: Value) -> Option<Self> {
        match value {
            Value::Float(value) => Some(value),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_conversions_check_the_target_width() {
        assert_eq!(i8::from_inject_value(Value::Signed(127)), Some(127));
        assert_eq!(i8::from_inject_value(Value::Signed(128)), None);
        assert_eq!(u8::from_inject_value(Value::Unsigned(256)), None);
        assert!(!ValueType::S32.accepts(&Value::Signed(i64::MAX)));
    }

    #[test]
    fn floats_keep_special_values_and_reject_finite_overflow() {
        assert_eq!(f32::from_inject_value(Value::Float(f64::INFINITY)), Some(f32::INFINITY));
        assert_eq!(f32::from_inject_value(Value::Float(f64::MAX)), None);
        assert!(f32::from_inject_value(Value::Float(f64::NAN)).unwrap().is_nan());
    }

    #[test]
    fn owned_text_and_bytes_round_trip_without_cross_conversion() {
        let text = String::from("hello");
        assert_eq!(String::from_inject_value(text.clone().into_inject_value()), Some(text));
        let bytes = vec![0, 127, 255];
        assert_eq!(Vec::<u8>::from_inject_value(bytes.clone().into_inject_value()), Some(bytes));
        assert_eq!(String::from_inject_value(Value::Bytes(vec![65])), None);
    }
}
