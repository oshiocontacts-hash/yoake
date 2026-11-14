use crate::{entities::Experience, health::PlayerHealth, mobs::Mob, motion::PlayerMotion};
use horizon_world::yoake_state_catalog::{describe, types};
use serde_json::{Value, json};

pub fn catalog() -> Value {
    let mut types = types();
    types.extend([
        describe::<Experience>(),
        describe::<PlayerMotion>(),
        describe::<Mob>(),
        describe::<PlayerHealth>(),
    ]);
    json!({ "schema": 1, "types": types })
}

#[cfg(test)]
mod tests {
    use super::catalog;
    use crate::entities::Experience;
    use yoake_state::{State, Value};

    #[test]
    fn generated_catalog_matches_committed_file() {
        let committed: serde_json::Value = serde_json::from_str(include_str!(
            "../../yoake-wasm/wit/state-catalog.json"
        ))
        .expect("state catalog must be valid JSON");
        assert_eq!(catalog(), committed);
    }

    #[test]
    fn experience_snapshot_is_read_only() {
        let mut experience = Experience {
            level: 12,
            progress: 0.5,
            total: 239,
        };
        assert_eq!(experience.get(&["level".into()]), Ok(Value::Signed(12)));
        assert!(experience.set(&["level".into()], &Value::Signed(13)).is_err());
        assert_eq!(experience.level, 12);
    }
}
