#![forbid(unsafe_code)]

extern crate self as yoake_state;

use std::collections::BTreeMap;

mod constants;
mod handles;
mod writes;

pub use constants::{ConstantDefinition, ConstantKind, ConstantOverrides, ConstantSnapshot};
pub use handles::{HandleTable, MAX_HANDLES, ResolvedHandle, StateHandle, StateObject};
pub use yoake_state_macro::{yoake_const, yoake_state};
pub use writes::{MAX_PENDING_WRITES, PendingWrites};

pub const MAX_PATH_DEPTH: usize = 32;
pub const MAX_PATH_COMPONENT_BYTES: usize = 128;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Boolean(bool),
    Signed(i64),
    Unsigned(u64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Record(BTreeMap<String, Value>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: &'static str,
    pub value_type: &'static str,
    pub writable: bool,
}

pub trait State {
    fn type_name() -> &'static str
    where
        Self: Sized;

    fn fields() -> &'static [Field]
    where
        Self: Sized;

    fn snapshot(&self) -> Result<Value, String>;

    fn validate_set(&self, path: &[String], value: &Value) -> Result<(), String>;

    fn set(&mut self, path: &[String], value: &Value) -> Result<(), String>;

    fn get(&self, path: &[String]) -> Result<Value, String> {
        get_path(&self.snapshot()?, path).cloned()
    }
}

pub fn validate_path(path: &[String]) -> Result<(), String> {
    if path.len() > MAX_PATH_DEPTH {
        return Err("state path is too deep".into());
    }
    if path
        .iter()
        .any(|part| part.is_empty() || part.len() > MAX_PATH_COMPONENT_BYTES)
    {
        return Err("invalid state path component".into());
    }
    Ok(())
}

pub fn get_path<'a>(value: &'a Value, path: &[String]) -> Result<&'a Value, String> {
    validate_path(path)?;
    let mut current = value;
    for component in path {
        current = match current {
            Value::Record(fields) => fields
                .get(component)
                .ok_or_else(|| format!("unknown state field: {component}"))?,
            Value::List(items) => {
                let index = component
                    .parse::<usize>()
                    .map_err(|_| "invalid state list index".to_owned())?;
                items
                    .get(index)
                    .ok_or_else(|| "state list index out of bounds".to_owned())?
            }
            _ => return Err("state path traverses a scalar".into()),
        };
    }
    Ok(current)
}

pub trait Scalar: Sized {
    fn into_state_value(self) -> Result<Value, String>;
    fn from_state_value(value: &Value) -> Result<Self, String>;
}

impl Scalar for bool {
    fn into_state_value(self) -> Result<Value, String> {
        Ok(Value::Boolean(self))
    }

    fn from_state_value(value: &Value) -> Result<Self, String> {
        match value {
            Value::Boolean(value) => Ok(*value),
            _ => Err("expected a boolean state value".into()),
        }
    }
}

impl Scalar for i32 {
    fn into_state_value(self) -> Result<Value, String> {
        Ok(Value::Signed(i64::from(self)))
    }

    fn from_state_value(value: &Value) -> Result<Self, String> {
        match value {
            Value::Signed(value) => i32::try_from(*value)
                .map_err(|_| "signed state value is outside i32 range".into()),
            _ => Err("expected a signed state value".into()),
        }
    }
}

impl Scalar for f32 {
    fn into_state_value(self) -> Result<Value, String> {
        if self.is_finite() {
            Ok(Value::Float(f64::from(self)))
        } else {
            Err("state value must be finite".into())
        }
    }

    fn from_state_value(value: &Value) -> Result<Self, String> {
        match value {
            Value::Float(value)
                if value.is_finite() && value.abs() <= f64::from(f32::MAX) =>
            {
                Ok(*value as f32)
            }
            Value::Float(_) => Err("floating-point state value is outside f32 range".into()),
            _ => Err("expected a floating-point state value".into()),
        }
    }
}

impl Scalar for f64 {
    fn into_state_value(self) -> Result<Value, String> {
        if self.is_finite() {
            Ok(Value::Float(self))
        } else {
            Err("state value must be finite".into())
        }
    }

    fn from_state_value(value: &Value) -> Result<Self, String> {
        match value {
            Value::Float(value) if value.is_finite() => Ok(*value),
            Value::Float(_) => Err("state value must be finite".into()),
            _ => Err("expected a floating-point state value".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{State, Value, get_path, yoake_state};
    use std::collections::BTreeMap;

    #[yoake_state]
    struct TestState {
        #[yoake_state(write, min = 0, max = 20)]
        food: i32,
        #[yoake_state(read_only)]
        ready: bool,
        #[yoake_state(skip)]
        private: i32,
        omitted: i32,
        #[yoake_state(write, min = 0.0, max = 1.0)]
        ratio: f32,
    }

    #[yoake_state]
    struct RawIdentifier {
        #[yoake_state(read_only)]
        r#type: i32,
    }

    #[yoake_state]
    struct ConditionalState {
        #[cfg(any())]
        #[yoake_state(write, min = 0, max = 20)]
        hidden: i32,
        #[cfg(all())]
        #[yoake_state(read_only)]
        visible: bool,
    }

    #[test]
    fn conditionally_disabled_fields_are_not_exposed() {
        let state = ConditionalState { visible: true };
        assert_eq!(ConditionalState::fields().len(), 1);
        assert_eq!(state.get(&["visible".into()]), Ok(Value::Boolean(true)));
        assert!(state.get(&["hidden".into()]).is_err());
    }

    #[test]
    fn raw_identifiers_use_normal_paths() {
        let state = RawIdentifier { r#type: 4 };
        assert_eq!(state.yoake_state_type(), 4);
        assert_eq!(state.get(&["type".into()]), Ok(Value::Signed(4)));
    }

    #[test]
    fn snapshots_only_expose_annotated_fields() {
        let state = TestState {
            food: 10,
            ready: true,
            private: 7,
            omitted: 12,
            ratio: 0.5,
        };
        assert_eq!(state.yoake_state_food(), 10);
        assert!(state.yoake_state_ready());
        assert_eq!(state.yoake_state_ratio(), 0.5);
        assert_eq!(state.get(&["ready".into()]), Ok(Value::Boolean(true)));
        assert!(state.get(&["private".into()]).is_err());
        assert!(state.get(&["omitted".into()]).is_err());
        assert_eq!(state.private, 7);
        assert_eq!(state.omitted, 12);
        assert_eq!(TestState::fields().len(), 3);
    }

    #[test]
    fn writes_are_checked_before_application() {
        let mut state = TestState {
            food: 10,
            ready: true,
            private: 7,
            omitted: 12,
            ratio: 0.5,
        };
        let food = ["food".into()];
        assert!(state.set(&food, &Value::Signed(21)).is_err());
        assert!(state.set(&food, &Value::Signed(i64::MAX)).is_err());
        assert!(state.set(&food, &Value::Boolean(false)).is_err());
        assert!(state.set(&["ready".into()], &Value::Boolean(false)).is_err());
        assert!(state.set(&["ratio".into()], &Value::Float(1.0 + f64::EPSILON)).is_err());
        assert!(state.set(&["ratio".into()], &Value::Float(f64::NAN)).is_err());
        assert_eq!(state.food, 10);
        assert_eq!(state.set(&food, &Value::Signed(20)), Ok(()));
        assert_eq!(state.food, 20);
    }

    #[test]
    fn paths_resolve_records_and_lists_without_panics() {
        let mut fields = BTreeMap::new();
        fields.insert("items".into(), Value::List(vec![Value::Signed(9)]));
        let root = Value::Record(fields);
        assert_eq!(
            get_path(&root, &["items".into(), "0".into()]),
            Ok(&Value::Signed(9))
        );
        assert!(get_path(&root, &["items".into(), "1".into()]).is_err());
        assert!(get_path(&root, &["items".into(), "x".into()]).is_err());
        assert!(get_path(&root, &["missing".into()]).is_err());
        assert!(get_path(&root, &vec!["a".into(); 33]).is_err());
    }

    #[test]
    fn non_finite_values_are_rejected() {
        use super::Scalar;
        assert!(f32::from_state_value(&Value::Float(f64::NAN)).is_err());
        assert!(f64::from_state_value(&Value::Float(f64::INFINITY)).is_err());
        assert!(f32::NAN.into_state_value().is_err());
    }
}
