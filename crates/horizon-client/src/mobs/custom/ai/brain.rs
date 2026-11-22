//! Brain option for custom mobs (`ai.memories`, `ai.sensors`, `ai.activities`, `ai.schedule`,
//! F4M-4b).
//!
//! Built over the engine's existing [`GoalSelector`] and [`Goal`] objects: goals belonging to an
//! activity are wrapped in [`ActivityGoal`], which gates `can_use` and `can_continue` on whether
//! that activity is currently active. Mobs without a brain (`CustomType::brain == None`) do not
//! wrap any goals and take no brain tick cost.
use super::{
    CustomType,
    memory::{MemoryBank, MemoryDef, MemoryKind, MemoryValue, value_from_json},
    sensors::{CompiledSensor, SensorState},
};
use crate::mobs::{
    Mob, MobWorld,
    goals::{Cx, Goal},
};
use horizon_mods::custom_mob::{CustomMobFile, Goal as GoalDef, MemoryPredicate};
use std::collections::BTreeMap;
use std::sync::Arc;

/// A compiled memory predicate in an activity's `when` list.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledPredicate {
    pub memory: String,
    pub present: Option<bool>,
    pub eq: Option<MemoryValue>,
    pub ne: Option<MemoryValue>,
    pub gt: Option<f64>,
    pub gte: Option<f64>,
    pub lt: Option<f64>,
    pub lte: Option<f64>,
}

impl CompiledPredicate {
    fn from_file(p: &MemoryPredicate, kind: MemoryKind) -> Result<Self, String> {
        let eq = match &p.eq {
            Some(v) => Some(
                value_from_json(kind, v)
                    .ok_or_else(|| format!("predicate on {}: bad eq value", p.memory))?,
            ),
            None => None,
        };
        let ne = match &p.ne {
            Some(v) => Some(
                value_from_json(kind, v)
                    .ok_or_else(|| format!("predicate on {}: bad ne value", p.memory))?,
            ),
            None => None,
        };
        let has_cmp = eq.is_some()
            || ne.is_some()
            || p.gt.is_some()
            || p.gte.is_some()
            || p.lt.is_some()
            || p.lte.is_some();
        let present = p.present.or_else(|| (!has_cmp).then_some(true));
        Ok(Self {
            memory: p.memory.clone(),
            present,
            eq,
            ne,
            gt: p.gt,
            gte: p.gte,
            lt: p.lt,
            lte: p.lte,
        })
    }

    pub fn eval(&self, bank: &MemoryBank) -> bool {
        let current = bank.get(&self.memory);
        match self.present {
            Some(false) => return current.is_none(),
            Some(true) if current.is_none() => return false,
            _ => {}
        }
        let Some(val) = current else {
            return false;
        };
        if let Some(expected) = &self.eq
            && val != expected
        {
            return false;
        }
        if let Some(forbidden) = &self.ne
            && val == forbidden
        {
            return false;
        }
        if self.gt.is_some() || self.gte.is_some() || self.lt.is_some() || self.lte.is_some() {
            let num = match val {
                MemoryValue::I32(n) => f64::from(*n),
                MemoryValue::F32(f) => f64::from(*f),
                _ => return false,
            };
            let ok = self.gt.is_none_or(|b| num > b)
                && self.gte.is_none_or(|b| num >= b)
                && self.lt.is_none_or(|b| num < b)
                && self.lte.is_none_or(|b| num <= b);
            if !ok {
                return false;
            }
        }
        true
    }
}

/// One compiled brain activity.
#[derive(Clone, Debug)]
pub struct ActivityDef {
    pub name: String,
    pub priority: i32,
    pub core: bool,
    pub when: Vec<CompiledPredicate>,
    pub goals: Vec<GoalDef>,
    pub targets: Vec<GoalDef>,
}

/// Shared immutable brain definition on [`CustomType`]. `None` when the mob has no brain.
#[derive(Clone, Debug)]
pub struct BrainDef {
    pub memories: Vec<MemoryDef>,
    pub memory_map: BTreeMap<String, usize>,
    pub sensors: Vec<CompiledSensor>,
    pub activities: Vec<ActivityDef>,
    /// Sorted by `time` (`0..24000`).
    pub schedule: Vec<(i32, String)>,
    pub default_activity: Option<String>,
}

impl BrainDef {
    /// Builds the brain definition from `file`, or `None` when the type uses plain goals only.
    pub fn from_file(file: &CustomMobFile) -> Result<Option<Self>, String> {
        let ai = &file.ai;
        let has_any = !ai.memories.is_empty()
            || !ai.sensors.is_empty()
            || !ai.activities.is_empty()
            || !ai.schedule.is_empty()
            || ai.default_activity.is_some()
            || ai.goals.iter().chain(&ai.targets).any(|g| g.activity.is_some());
        if !has_any {
            return Ok(None);
        }
        let mut memories = Vec::new();
        let mut memory_map = BTreeMap::new();
        for (name, def) in &ai.memories {
            let compiled = MemoryDef::from_file(name, def)?;
            memory_map.insert(name.clone(), memories.len());
            memories.push(compiled);
        }
        let mut sensors = Vec::with_capacity(ai.sensors.len());
        for s in &ai.sensors {
            let mem_name = s
                .resolved_memory_name()
                .ok_or_else(|| format!("sensor {}: missing memory name", s.kind))?;
            let (kind, default_ttl) = if let Some(&idx) = memory_map.get(mem_name) {
                (memories[idx].kind, memories[idx].ttl)
            } else {
                let kind_str = s
                    .default_memory_type()
                    .ok_or_else(|| format!("sensor {}: unknown type", s.kind))?;
                let kind = MemoryKind::parse(kind_str).unwrap();
                memory_map.insert(mem_name.to_owned(), memories.len());
                memories.push(MemoryDef {
                    name: mem_name.to_owned(),
                    kind,
                    default: None,
                    persistent: false,
                    ttl: None,
                });
                (kind, None)
            };
            sensors.push(CompiledSensor::from_file(s, kind, default_ttl)?);
        }
        let mut activities = Vec::with_capacity(ai.activities.len());
        for (name, act) in &ai.activities {
            let mut when = Vec::with_capacity(act.when.len());
            for p in &act.when {
                let idx = *memory_map.get(&p.memory).ok_or_else(|| {
                    format!("activity {name}: undeclared memory {:?}", p.memory)
                })?;
                when.push(CompiledPredicate::from_file(p, memories[idx].kind)?);
            }
            activities.push(ActivityDef {
                name: name.clone(),
                priority: act.priority,
                core: act.core,
                when,
                goals: act.goals.clone(),
                targets: act.targets.clone(),
            });
        }
        let mut schedule: Vec<(i32, String)> = ai
            .schedule
            .iter()
            .map(|e| (e.time, e.activity.clone()))
            .collect();
        schedule.sort_by_key(|(t, _)| *t);
        Ok(Some(Self {
            memories,
            memory_map,
            sensors,
            activities,
            schedule,
            default_activity: ai.default_activity.clone(),
        }))
    }

    pub fn memory_def(&self, name: &str) -> Option<&MemoryDef> {
        self.memory_map.get(name).map(|&i| &self.memories[i])
    }

    /// Which scheduled activity applies at `day_time` (if `schedule` is non-empty).
    pub fn scheduled_at(&self, day_time: i64) -> Option<&str> {
        if self.schedule.is_empty() {
            return None;
        }
        let tod = day_time.rem_euclid(24000) as i32;
        self.schedule
            .iter()
            .rev()
            .find(|(t, _)| *t <= tod)
            .or_else(|| self.schedule.last())
            .map(|(_, name)| name.as_str())
    }
}

/// Per-mob brain runtime state (`CustomState.brain`).
#[derive(Clone, Debug, Default)]
pub struct BrainState {
    pub memories: MemoryBank,
    pub sensors: SensorState,
    pub active_activity: Option<String>,
    pub active_cores: Vec<String>,
}

impl BrainState {
    pub fn new(t: &CustomType) -> Self {
        let Some(def) = &t.brain else {
            return Self::default();
        };
        let mut state = Self {
            memories: MemoryBank::from_defs(&def.memories),
            sensors: SensorState::new(def.sensors.len()),
            active_activity: None,
            active_cores: Vec::new(),
        };
        state.update_activity(def, 0);
        state
    }

    /// Evaluates core activities and picks the active non-core activity from predicates and schedule.
    pub fn update_activity(&mut self, def: &BrainDef, day_time: i64) {
        self.active_cores.clear();
        for a in def.activities.iter().filter(|a| a.core) {
            if a.when.iter().all(|p| p.eval(&self.memories)) {
                self.active_cores.push(a.name.clone());
            }
        }
        let scheduled = def.scheduled_at(day_time);
        let fallback_name = def.default_activity.as_deref().or_else(|| {
            def.activities
                .iter()
                .any(|a| !a.core && a.name == "idle" && a.when.is_empty())
                .then_some("idle")
        });

        // Eligible activities: those with passing `when` predicates (non-empty `when`), OR the
        // currently scheduled activity (whose `when`, if any, must also pass).
        let mut best: Option<(i32, bool, bool, &str)> = None;
        for a in def.activities.iter().filter(|a| !a.core) {
            let predicates_ok = a.when.iter().all(|p| p.eval(&self.memories));
            if !predicates_ok {
                continue;
            }
            let is_scheduled = scheduled == Some(a.name.as_str());
            if a.when.is_empty() && !is_scheduled {
                continue;
            }
            let cand = (a.priority, a.when.is_empty(), !is_scheduled, a.name.as_str());
            if best.is_none_or(|b| cand < b) {
                best = Some(cand);
            }
        }

        if let Some((_, _, _, chosen)) = best {
            self.active_activity = Some(chosen.to_owned());
            return;
        }
        // Fallback when no gated/scheduled activity matched: `default_activity`, else `"idle"`,
        // else the lowest-priority non-core activity with empty `when`.
        let mut fallback: Option<(i32, &str)> = None;
        if let Some(name) = fallback_name
            && let Some(a) = def.activities.iter().find(|a| !a.core && a.name == name)
            && a.when.iter().all(|p| p.eval(&self.memories))
        {
            self.active_activity = Some(a.name.clone());
            return;
        }
        for a in def.activities.iter().filter(|a| !a.core && a.when.is_empty()) {
            let cand = (a.priority, a.name.as_str());
            if fallback.is_none_or(|b| cand < b) {
                fallback = Some(cand);
            }
        }
        self.active_activity = fallback.map(|(_, n)| n.to_owned());
    }

    pub fn is_active(&self, activity: &str) -> bool {
        self.active_activity.as_deref() == Some(activity)
            || self.active_cores.iter().any(|c| c == activity)
    }
}

/// Whether `activity` is currently active on `m`.
pub fn is_active(m: &Mob, activity: &str) -> bool {
    m.extra
        .custom
        .as_ref()
        .is_some_and(|c| c.brain.is_active(activity))
}

/// Currently active non-core activity of `m`, if any.
pub fn active_activity(m: &Mob) -> Option<&str> {
    m.extra
        .custom
        .as_ref()
        .and_then(|c| c.brain.active_activity.as_deref())
}

/// Step 1 of the brain tick (before scripts): expires outdated memories and runs due sensors.
pub fn tick_pre(m: &mut Mob, w: &MobWorld) {
    let Some(t) = m.kind.custom.clone() else {
        return;
    };
    let Some(def) = &t.brain else {
        return;
    };
    let Some(mut brain) = m
        .extra
        .custom
        .as_mut()
        .map(|c| std::mem::take(&mut c.brain))
    else {
        return;
    };
    brain.memories.forget_outdated();
    super::sensors::tick(m, w, &def.sensors, &mut brain.sensors, &mut brain.memories);
    if let Some(c) = m.extra.custom.as_mut() {
        c.brain = brain;
    }
}

/// Step 2 of the brain tick (after scripts, before goals): switches active activity from memories
/// and schedule.
pub fn update_activity(m: &mut Mob, w: &MobWorld) {
    let Some(t) = m.kind.custom.clone() else {
        return;
    };
    let Some(def) = &t.brain else {
        return;
    };
    if let Some(c) = m.extra.custom.as_mut() {
        let prev = c.brain.active_activity.clone();
        c.brain.update_activity(def, w.day_time);
        if c.brain.active_activity != prev {
            log::info!(
                "Horizon custom-mob brain: mob {} activity {:?} -> {:?} tick {}",
                m.id,
                prev,
                c.brain.active_activity,
                w.game_time
            );
        }
    }
}

/// Runs both brain tick steps in order (helper for tests and callers outside `Mob::tick`).
pub fn tick(m: &mut Mob, w: &MobWorld) {
    tick_pre(m, w);
    update_activity(m, w);
}

/// Wraps any [`Goal`] so it only runs while `activity` is active on the mob's brain.
#[derive(Clone, Debug)]
pub struct ActivityGoal {
    activity: Arc<str>,
    inner: Box<dyn Goal>,
}

impl ActivityGoal {
    pub fn new(activity: Arc<str>, inner: Box<dyn Goal>) -> Self {
        Self { activity, inner }
    }
}

impl Goal for ActivityGoal {
    fn box_clone(&self) -> Box<dyn Goal> {
        Box::new(self.clone())
    }
    fn name(&self) -> &'static str {
        self.inner.name()
    }
    fn flags(&self) -> u8 {
        self.inner.flags()
    }
    fn set_start_delay(&mut self, ticks: i32) {
        self.inner.set_start_delay(ticks);
    }
    fn interruptable(&self) -> bool {
        self.inner.interruptable()
    }
    fn every_tick(&self) -> bool {
        self.inner.every_tick()
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        is_active(m, &self.activity) && self.inner.can_use(m, cx)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        is_active(m, &self.activity) && self.inner.can_continue(m, cx)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.inner.start(m, cx);
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.inner.tick(m, cx);
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        self.inner.stop(m, cx);
    }
}
