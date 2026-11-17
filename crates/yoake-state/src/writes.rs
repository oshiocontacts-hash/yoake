use crate::{ResolvedHandle, State, Value, validate_path};
use std::sync::Arc;

pub const MAX_PENDING_WRITES: usize = 128;
const MAX_VALUE_NODES: usize = 4096;
const MAX_VALUE_BYTES: usize = 65536;

#[derive(Clone, Debug)]
pub(crate) struct WriteTarget {
    pub(crate) table: Arc<()>,
    pub(crate) instance: u64,
    pub(crate) world_generation: u64,
    pub(crate) object_generation: u64,
    pub(crate) handle: u64,
}

impl PartialEq for WriteTarget {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.table, &other.table)
            && self.instance == other.instance
            && self.world_generation == other.world_generation
            && self.object_generation == other.object_generation
            && self.handle == other.handle
    }
}

impl Eq for WriteTarget {}

struct Write {
    path: Vec<String>,
    value: Value,
}

pub struct PendingWrites {
    target: WriteTarget,
    state_type: &'static str,
    writes: Vec<Write>,
}

impl PendingWrites {
    pub fn new<T: State>(target: &ResolvedHandle<'_>, _state: &T) -> Self {
        Self {
            target: target.write_target(),
            state_type: T::type_name(),
            writes: Vec::new(),
        }
    }

    pub fn stage<T: State>(
        &mut self,
        target: &ResolvedHandle<'_>,
        state: &T,
        path: Vec<String>,
        value: Value,
    ) -> Result<(), String> {
        self.check_target::<T>(target.write_target())?;
        if self.writes.len() >= MAX_PENDING_WRITES {
            return Err("too many pending state writes".into());
        }
        validate_path(&path)?;
        validate_value(&value)?;
        state.validate_set(&path, &value)?;
        self.writes.push(Write { path, value });
        Ok(())
    }

    pub fn apply<T: State>(
        &mut self,
        target: &ResolvedHandle<'_>,
        state: &mut T,
    ) -> Result<Vec<Result<(), String>>, String> {
        self.check_target::<T>(target.write_target())?;
        Ok(self
            .writes
            .drain(..)
            .map(|write| {
                state.validate_set(&write.path, &write.value)?;
                state.set(&write.path, &write.value)
            })
            .collect())
    }

    pub fn discard(&mut self) {
        self.writes.clear();
    }

    fn check_target<T: State>(&self, target: WriteTarget) -> Result<(), String> {
        if target != self.target || T::type_name() != self.state_type {
            return Err("state write target is stale or belongs to another scope".into());
        }
        Ok(())
    }
}

fn validate_value(value: &Value) -> Result<(), String> {
    let mut remaining_nodes = MAX_VALUE_NODES;
    let mut remaining_bytes = MAX_VALUE_BYTES;
    let mut pending = vec![(value, 0)];
    while let Some((value, depth)) = pending.pop() {
        if depth > crate::MAX_PATH_DEPTH || remaining_nodes == 0 {
            return Err("state value is too complex".into());
        }
        remaining_nodes -= 1;
        let size = match value {
            Value::Float(number) if !number.is_finite() => {
                return Err("state value must be finite".into());
            }
            Value::Text(text) => text.len(),
            Value::Bytes(bytes) => bytes.len(),
            Value::List(values) => {
                if values.len() > remaining_nodes.saturating_sub(pending.len()) {
                    return Err("state value is too complex".into());
                }
                pending.extend(values.iter().map(|child| (child, depth + 1)));
                0
            }
            Value::Record(fields) => {
                if fields.len() > remaining_nodes.saturating_sub(pending.len()) {
                    return Err("state value is too complex".into());
                }
                let mut size = 0usize;
                for (key, child) in fields {
                    size = size.checked_add(key.len()).ok_or("state value is too large")?;
                    pending.push((child, depth + 1));
                }
                size
            }
            _ => 0,
        };
        remaining_bytes = remaining_bytes
            .checked_sub(size)
            .ok_or("state value is too large")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MAX_PENDING_WRITES, PendingWrites, validate_value};
    use crate::{HandleTable, State, StateHandle, StateObject, Value, yoake_state};

    #[yoake_state]
    struct Example {
        #[yoake_state(write, min = 0, max = 20)]
        amount: i32,
    }

    #[yoake_state]
    struct ChangingLimit {
        #[yoake_state(write, min = 0, max = self.limit)]
        amount: i32,
        limit: i32,
    }

    fn table_and_handle() -> (HandleTable, StateHandle) {
        let mut table = HandleTable::default();
        let handle = table.issue(1, 2, 3, StateObject::Player { uuid: [4; 16] }).unwrap();
        (table, handle)
    }

    #[test]
    fn writes_wait_for_apply_and_reject_wrong_scope() {
        let mut state = Example { amount: 1 };
        assert_eq!(state.yoake_state_amount(), 1);
        let (mut table, handle) = table_and_handle();
        let other = table.issue(1, 2, 4, StateObject::Player { uuid: [4; 16] }).unwrap();
        let target = table.resolve(handle, 1, 2).unwrap();
        let stale = table.resolve(other, 1, 2).unwrap();
        let mut pending = PendingWrites::new(&target, &state);
        pending.stage(&target, &state, vec!["amount".into()], Value::Signed(7)).unwrap();
        assert_eq!(state.amount, 1);
        assert!(pending.apply(&stale, &mut state).is_err());
        assert_eq!(state.amount, 1);
        assert_eq!(pending.apply(&target, &mut state).unwrap(), vec![Ok(())]);
        assert_eq!(state.amount, 7);
        assert!(pending.apply(&target, &mut state).unwrap().is_empty());
    }

    #[test]
    fn writes_are_revalidated_against_the_current_state() {
        let mut state = ChangingLimit { amount: 1, limit: 20 };
        assert_eq!(state.yoake_state_amount(), 1);
        let (table, handle) = table_and_handle();
        let target = table.resolve(handle, 1, 2).unwrap();
        let mut pending = PendingWrites::new(&target, &state);
        pending.stage(&target, &state, vec!["amount".into()], Value::Signed(7)).unwrap();
        state.limit = 3;
        let outcomes = pending.apply(&target, &mut state).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].is_err());
        assert_eq!(state.amount, 1);
    }

    #[test]
    fn rejected_and_discarded_writes_never_apply() {
        let mut state = Example { amount: 1 };
        let (table, handle) = table_and_handle();
        let target = table.resolve(handle, 1, 2).unwrap();
        let mut pending = PendingWrites::new(&target, &state);
        assert!(pending.stage(&target, &state, vec!["amount".into()], Value::Signed(21)).is_err());
        pending.stage(&target, &state, vec!["amount".into()], Value::Signed(5)).unwrap();
        pending.discard();
        assert!(pending.apply(&target, &mut state).unwrap().is_empty());
        assert_eq!(state.amount, 1);
        assert!(state.validate_set(&["amount".into()], &Value::Signed(21)).is_err());
    }

    #[test]
    fn queue_and_values_are_bounded() {
        let state = Example { amount: 1 };
        let (table, handle) = table_and_handle();
        let target = table.resolve(handle, 1, 2).unwrap();
        let mut pending = PendingWrites::new(&target, &state);
        for _ in 0..MAX_PENDING_WRITES {
            pending.stage(&target, &state, vec!["amount".into()], Value::Signed(2)).unwrap();
        }
        assert!(pending.stage(&target, &state, vec!["amount".into()], Value::Signed(2)).is_err());
        assert!(validate_value(&Value::Text("a".repeat(65537))).is_err());
        assert!(validate_value(&Value::List(vec![Value::Float(f64::NAN)])).is_err());
        assert!(validate_value(&Value::List(vec![Value::Boolean(true); 4096])).is_err());
    }

    #[test]
    fn invalidated_handle_cannot_be_resolved_to_apply_pending_writes() {
        let state = Example { amount: 1 };
        let (mut table, handle) = table_and_handle();
        let mut pending = {
            let target = table.resolve(handle, 1, 2).unwrap();
            let mut pending = PendingWrites::new(&target, &state);
            pending.stage(&target, &state, vec!["amount".into()], Value::Signed(5)).unwrap();
            pending
        };
        table.invalidate(handle, 1).unwrap();
        assert!(table.resolve(handle, 1, 2).is_err());
        pending.discard();
        assert_eq!(state.amount, 1);
    }

    #[test]
    fn handles_from_different_tables_cannot_apply_the_same_queue() {
        let mut state = Example { amount: 1 };
        let (first_table, first_handle) = table_and_handle();
        let (second_table, second_handle) = table_and_handle();
        assert_eq!(first_handle, second_handle);
        let first = first_table.resolve(first_handle, 1, 2).unwrap();
        let second = second_table.resolve(second_handle, 1, 2).unwrap();
        let mut pending = PendingWrites::new(&first, &state);
        pending.stage(&first, &state, vec!["amount".into()], Value::Signed(5)).unwrap();
        assert!(pending.stage(&second, &state, vec!["amount".into()], Value::Signed(6)).is_err());
        assert!(pending.apply(&second, &mut state).is_err());
        assert_eq!(state.amount, 1);
        assert_eq!(pending.apply(&first, &mut state).unwrap(), vec![Ok(())]);
        assert_eq!(state.amount, 5);
    }
}
