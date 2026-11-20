//! The `when` expression of a goal row (FR-F4-MOBS.md section 3.6, the safe expression
//! language): compiled once when the mob is made, evaluated on the server against the mob's own
//! state. The inputs are the engine's: `query.health`, `max_health`, `is_on_ground`, `is_in_water`,
//! `is_baby`, `hurt_time`, `has_target`, `is_riding`, `life_time` (seconds), `time_of_day` (the
//! day time over 24000, 0 to 1) and `query.synced('name')`. The animation-only
//! inputs (`anim_time`, speeds, rotations, `death_time`) read 0 here.
use super::CustomType;
use crate::mobs::{Mob, MobWorld};
use horizon_mobmodel::expr::{Expr, ExprInputs, Query, Symbols, compile};

use super::super::SyncedValue;

/// A compiled `when` expression.
#[derive(Clone, Debug)]
pub struct When(Expr);

impl When {
    /// Compiles `src` against the synced values of `t`; the error text for a bad expression.
    pub fn compile(t: &CustomType, src: &str) -> Result<Self, String> {
        let names: Vec<&str> = t.synced.iter().map(|s| s.name.as_str()).collect();
        let mut syms = Symbols::new(&names);
        for s in t.synced.iter().filter(|s| s.kind == "enum") {
            let values: Vec<&str> = s.values.iter().map(String::as_str).collect();
            syms.set_enum_values(&s.name, &values);
        }
        compile(src, &mut syms).map(Self).map_err(|e| e.to_string())
    }

    /// Whether the expression is true for `m` now.
    pub fn eval(&self, m: &Mob, w: &MobWorld) -> bool {
        self.0.eval_bool(&inputs(m, w))
    }
}

/// What the expressions of a server mob read.
pub fn inputs(m: &Mob, w: &MobWorld) -> ExprInputs {
    let mut i = ExprInputs::default();
    fill(&mut i, m, w);
    i
}

/// Writes what the expressions of a server mob read into `i` (its `variables` are left alone: the
/// animation controllers keep theirs from tick to tick).
pub fn fill(i: &mut ExprInputs, m: &Mob, w: &MobWorld) {
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
    i.set(Query::Health, m.health);
    i.set(Query::MaxHealth, m.attrs.max_health);
    i.set(Query::IsOnGround, flag(m.on_ground));
    i.set(Query::IsInWater, flag(m.fluids.in_water));
    i.set(Query::IsBaby, flag(m.is_baby()));
    i.set(Query::HurtTime, m.hurt_time as f32);
    i.set(Query::HasTarget, flag(m.ai.target.is_some()));
    i.set(Query::IsRiding, flag(m.extra.mount.is_some()));
    i.set(Query::LifeTime, m.age as f32 / 20.0);
    i.set(Query::TimeOfDay, w.day_time.rem_euclid(24000) as f32 / 24000.0);
    i.seed = m.id;
    i.random_epoch = w.game_time as u32;
    if let Some(c) = &m.extra.custom {
        for (slot, v) in c.synced.iter().take(i.synced.len()).enumerate() {
            i.synced[slot] = match v {
                SyncedValue::Bool(b) => flag(*b),
                SyncedValue::I32(n) => *n as f32,
                SyncedValue::F32(f) => *f,
                SyncedValue::Enum(e) => f32::from(*e),
                SyncedValue::Str(_) | SyncedValue::Vec3(_) => 0.0,
            };
        }
    }
}
