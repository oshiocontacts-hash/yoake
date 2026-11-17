use std::collections::BTreeMap;

use crate::{Scalar, Value};

const MAX_CONSTANTS: usize = 4096;
const MAX_ID_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstantKind {
    Boolean,
    I32,
    F32,
    F64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConstantDefinition {
    id: &'static str,
    kind: ConstantKind,
    default: Value,
    minimum: Option<Value>,
    maximum: Option<Value>,
}

impl ConstantDefinition {
    pub fn new(
        id: &'static str,
        kind: ConstantKind,
        default: Value,
        minimum: Option<Value>,
        maximum: Option<Value>,
    ) -> Result<Self, String> {
        if id.is_empty()
            || id.len() > MAX_ID_BYTES
            || !id.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':')
            })
        {
            return Err("invalid constant identifier".into());
        }
        match (kind, &minimum, &maximum) {
            (ConstantKind::Boolean, None, None) => {}
            (ConstantKind::I32 | ConstantKind::F32 | ConstantKind::F64, Some(_), Some(_)) => {}
            _ => return Err("numeric constants require bounds and booleans cannot have them".into()),
        }
        let definition = Self { id, kind, default, minimum, maximum };
        if let (Some(minimum), Some(maximum)) = (&definition.minimum, &definition.maximum) {
            definition.validate_type(minimum)?;
            definition.validate_type(maximum)?;
            if !ordered(minimum, maximum) {
                return Err("constant bounds are invalid".into());
            }
        }
        definition.validate(&definition.default)?;
        Ok(definition)
    }

    pub fn id(&self) -> &'static str {
        self.id
    }

    pub fn validate(&self, value: &Value) -> Result<(), String> {
        self.validate_type(value)?;
        if let (Some(minimum), Some(maximum)) = (&self.minimum, &self.maximum) {
            if !ordered(minimum, value) || !ordered(value, maximum) {
                return Err("constant override is outside its permitted range".into());
            }
        }
        Ok(())
    }

    fn validate_type(&self, value: &Value) -> Result<(), String> {
        match self.kind {
            ConstantKind::Boolean => bool::from_state_value(value).map(|_| ()),
            ConstantKind::I32 => i32::from_state_value(value).map(|_| ()),
            ConstantKind::F32 => f32::from_state_value(value).map(|_| ()),
            ConstantKind::F64 => f64::from_state_value(value).map(|_| ()),
        }
    }
}

fn ordered(first: &Value, second: &Value) -> bool {
    match (first, second) {
        (Value::Signed(first), Value::Signed(second)) => first <= second,
        (Value::Float(first), Value::Float(second)) => first <= second,
        _ => false,
    }
}

#[derive(Default)]
pub struct ConstantOverrides {
    definitions: BTreeMap<&'static str, ConstantDefinition>,
    overrides: BTreeMap<&'static str, Value>,
}

impl ConstantOverrides {
    pub fn register(&mut self, definition: ConstantDefinition) -> Result<(), String> {
        if self.definitions.contains_key(definition.id) {
            return Err("duplicate constant identifier".into());
        }
        if self.definitions.len() >= MAX_CONSTANTS {
            return Err("constant registry is full".into());
        }
        self.definitions.insert(definition.id, definition);
        Ok(())
    }

    pub fn override_value(&mut self, id: &str, value: Value) -> Result<(), String> {
        let (key, definition) = self.definitions.get_key_value(id)
            .ok_or_else(|| "unknown constant identifier".to_owned())?;
        definition.validate(&value)?;
        self.overrides.insert(*key, value);
        Ok(())
    }

    pub fn reset(&mut self, id: &str) -> Result<(), String> {
        if !self.definitions.contains_key(id) {
            return Err("unknown constant identifier".into());
        }
        self.overrides.remove(id);
        Ok(())
    }

    pub fn reset_all(&mut self) {
        self.overrides.clear();
    }

    pub fn snapshot(&self) -> ConstantSnapshot {
        ConstantSnapshot {
            values: self.definitions.iter().map(|(id, definition)| {
                (*id, self.overrides.get(id).unwrap_or(&definition.default).clone())
            }).collect(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConstantSnapshot {
    values: BTreeMap<&'static str, Value>,
}

impl ConstantSnapshot {
    pub fn get(&self, id: &str) -> Option<&Value> {
        self.values.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::ConstantOverrides;
    use crate::{Value, yoake_const};

    #[yoake_const(id = "test.speed", min = 0.0, max = 10.0)]
    const SPEED: f32 = 2.5;

    #[yoake_const(id = "test.enabled")]
    const ENABLED: bool = true;

    #[test]
    fn overrides_are_validated_and_snapshots_remain_stable() {
        let mut constants = ConstantOverrides::default();
        constants.register(yoake_const_speed().unwrap()).unwrap();
        constants.register(yoake_const_enabled().unwrap()).unwrap();
        assert!(constants.register(yoake_const_speed().unwrap()).is_err());
        let before = constants.snapshot();
        constants.override_value("test.speed", Value::Float(5.0)).unwrap();
        assert_eq!(before.get("test.speed"), Some(&Value::Float(2.5)));
        assert_eq!(constants.snapshot().get("test.speed"), Some(&Value::Float(5.0)));
        for invalid in [Value::Float(f64::NAN), Value::Float(10.1), Value::Signed(3)] {
            assert!(constants.override_value("test.speed", invalid).is_err());
        }
        assert!(constants.override_value("missing", Value::Float(1.0)).is_err());
        assert_eq!(constants.snapshot().get("test.speed"), Some(&Value::Float(5.0)));
        constants.reset("test.speed").unwrap();
        assert_eq!(constants.snapshot().get("test.speed"), Some(&Value::Float(2.5)));
        constants.override_value("test.enabled", Value::Boolean(false)).unwrap();
        constants.reset_all();
        assert_eq!(constants.snapshot().get("test.enabled"), Some(&Value::Boolean(true)));
    }
}
