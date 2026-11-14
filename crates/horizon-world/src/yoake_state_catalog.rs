use crate::{
    food::FoodData,
    level::{
        GameRules, pistons::MovingPiston, portal_travel::BorderBounds, sculk::ChargeCursor,
    },
    status_effect::Details,
};
use serde_json::{Value, json};
use yoake_state::State;

pub fn describe<T: State>() -> Value {
    let fields: Vec<_> = T::fields()
        .iter()
        .map(|field| {
            json!({
                "name": field.name,
                "type": field.value_type,
                "writable": field.writable,
            })
        })
        .collect();
    json!({ "type": T::type_name(), "fields": fields })
}

pub fn types() -> Vec<Value> {
    vec![
        describe::<FoodData>(),
        describe::<GameRules>(),
        describe::<BorderBounds>(),
        describe::<MovingPiston>(),
        describe::<ChargeCursor>(),
        describe::<Details>(),
    ]
}

#[cfg(test)]
mod tests {
    use crate::{
        food::FoodData,
        level::{pistons::MovingPiston, sculk::ChargeCursor},
        registry::BlockStateId,
    };
    use horizon_core::{geometry::Direction, position::BlockPos};
    use yoake_state::{State, Value};

    #[test]
    fn food_snapshot_is_read_only() {
        let mut food = FoodData::default();
        assert_eq!(food.get(&["food".into()]), Ok(Value::Signed(20)));
        assert!(food.set(&["food".into()], &Value::Signed(10)).is_err());
        assert_eq!(food.food, 20);
    }

    #[test]
    fn moving_piston_snapshot_is_read_only() {
        let mut piston = MovingPiston {
            moved_state: BlockStateId(42),
            direction: Direction::North,
            extending: true,
            source: false,
            progress: 0.5,
            progress_o: 0.25,
            last_ticked: 5,
            death_ticks: 2,
            id: 7,
        };
        assert_eq!(piston.get(&["progress".into()]), Ok(Value::Float(0.5)));
        assert!(piston.set(&["progress".into()], &Value::Float(1.0)).is_err());
        assert_eq!(piston.progress, 0.5);
        assert!(piston.get(&["moved_state".into()]).is_err());
    }

    #[test]
    fn sculk_cursor_snapshot_is_read_only() {
        let mut cursor = ChargeCursor::new(BlockPos::new(1, 2, 3), 300);
        assert_eq!(cursor.get(&["charge".into()]), Ok(Value::Signed(300)));
        assert_eq!(cursor.get(&["decay_delay".into()]), Ok(Value::Signed(1)));
        assert!(cursor.set(&["charge".into()], &Value::Signed(10)).is_err());
        assert_eq!(cursor.charge, 300);
    }
}
