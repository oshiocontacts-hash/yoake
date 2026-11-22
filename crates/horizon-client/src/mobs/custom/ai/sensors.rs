//! Declarative brain sensors (`ai.sensors`, F4M-4b).
//!
//! Supported sensor types:
//! - `nearest_players` (`range`, `must_see`, `ignore_creative`) -> `entity`
//! - `nearest_living` (`types`, `range`, `must_see`) -> `entity`
//! - `hurt_by` (`max_age`) -> `entity` or `bool`
//! - `blocks` (`tag`, `blocks`, `radius`, `vertical_radius`) -> `block_pos` or `vec3`
//! - `light` (`kind`: `combined` | `sky` | `block`) -> `i32`, `f32`, or `bool`
use super::{
    TypeList,
    memory::{EntityMemory, MemoryBank, MemoryKind, MemoryValue},
    type_list,
};
use crate::mobs::{Mob, MobWorld, Who, goals::seen_is_type};
use horizon_mods::custom_mob::Sensor as FileSensor;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightKind {
    Combined,
    Sky,
    Block,
}

#[derive(Clone, Debug)]
pub enum SensorKind {
    NearestPlayers {
        range: Option<f64>,
        must_see: bool,
        ignore_creative: bool,
    },
    NearestLiving {
        types: Option<TypeList>,
        range: Option<f64>,
        must_see: bool,
    },
    HurtBy {
        max_age: Option<i64>,
    },
    Blocks {
        tag: Option<Arc<str>>,
        blocks: Vec<String>,
        radius: i32,
        vertical_radius: i32,
    },
    Light {
        kind: LightKind,
    },
}

/// A compiled sensor definition on `BrainDef`.
#[derive(Clone, Debug)]
pub struct CompiledSensor {
    pub kind: SensorKind,
    pub interval: u32,
    pub memory: String,
    pub memory_kind: MemoryKind,
    pub ttl: Option<i32>,
}

impl CompiledSensor {
    pub fn from_file(s: &FileSensor, memory_kind: MemoryKind, default_ttl: Option<i32>) -> Result<Self, String> {
        let memory = s
            .resolved_memory_name()
            .ok_or_else(|| format!("sensor {}: missing memory name", s.kind))?
            .to_owned();
        let ttl = s
            .ttl
            .map(|t| i32::try_from(t).unwrap_or(i32::MAX))
            .or(default_ttl);
        let kind = match s.kind.as_str() {
            "nearest_players" => SensorKind::NearestPlayers {
                range: s.range,
                must_see: s.must_see.unwrap_or(true),
                ignore_creative: s.ignore_creative.unwrap_or(true),
            },
            "nearest_living" => {
                let types = if s.types.is_empty() {
                    None
                } else {
                    Some(type_list(&s.types, "sensor nearest_living"))
                };
                SensorKind::NearestLiving {
                    types,
                    range: s.range,
                    must_see: s.must_see.unwrap_or(true),
                }
            }
            "hurt_by" => SensorKind::HurtBy {
                max_age: s.max_age.map(i64::from),
            },
            "blocks" => {
                let radius = s.radius.unwrap_or(8).clamp(1, 32);
                let vertical_radius = s.vertical_radius.unwrap_or_else(|| radius.min(8)).clamp(0, 16);
                let tag = s
                    .tag
                    .as_deref()
                    .map(|t| t.strip_prefix('#').unwrap_or(t))
                    .filter(|t| !t.is_empty())
                    .map(Arc::<str>::from);
                SensorKind::Blocks {
                    tag,
                    blocks: s.blocks.clone(),
                    radius,
                    vertical_radius,
                }
            }
            "light" => {
                let kind = match s.light_kind.as_deref().unwrap_or("combined") {
                    "sky" => LightKind::Sky,
                    "block" => LightKind::Block,
                    _ => LightKind::Combined,
                };
                SensorKind::Light { kind }
            }
            other => return Err(format!("unknown sensor type {other:?}")),
        };
        Ok(Self {
            kind,
            interval: s.interval.clamp(1, 2400),
            memory,
            memory_kind,
            ttl,
        })
    }
}

/// Per-mob runtime state for sensors.
#[derive(Clone, Debug, Default)]
pub struct SensorState {
    pub cooldowns: Vec<u32>,
    pub last_damage_at: Vec<Option<i64>>,
}

impl SensorState {
    pub fn new(count: usize) -> Self {
        Self {
            cooldowns: vec![0; count],
            last_damage_at: vec![None; count],
        }
    }
}

/// Ticks all sensors on `m`, firing each one when its interval reaches zero.
pub fn tick(
    m: &mut Mob,
    w: &MobWorld,
    sensors: &[CompiledSensor],
    state: &mut SensorState,
    memories: &mut MemoryBank,
) {
    if state.cooldowns.len() != sensors.len() {
        state.cooldowns.resize(sensors.len(), 0);
    }
    if state.last_damage_at.len() != sensors.len() {
        state.last_damage_at.resize(sensors.len(), None);
    }
    for (i, sensor) in sensors.iter().enumerate() {
        if state.cooldowns[i] > 0 {
            state.cooldowns[i] -= 1;
            continue;
        }
        state.cooldowns[i] = sensor.interval.saturating_sub(1);
        run_one(m, w, sensor, &mut state.last_damage_at[i], memories);
    }
}

fn run_one(
    m: &mut Mob,
    w: &MobWorld,
    sensor: &CompiledSensor,
    last_damage_at: &mut Option<i64>,
    memories: &mut MemoryBank,
) {
    match &sensor.kind {
        SensorKind::NearestPlayers {
            range,
            must_see,
            ignore_creative,
        } => {
            let r = range.unwrap_or(m.attrs.follow_range).clamp(1.0, 128.0);
            let found = w
                .player
                .as_ref()
                .filter(|p| {
                    p.alive
                        && !p.spectator
                        && (!ignore_creative || !p.creative)
                        && m.distance_sqr(p.position) <= r * r
                })
                .map(|p| p.uuid)
                .filter(|_| !must_see || m.sees(w, Who::Player));
            write_optional(memories, &sensor.memory, sensor.ttl, found.map(|uuid| MemoryValue::Entity(EntityMemory::Player(uuid))));
        }
        SensorKind::NearestLiving {
            types,
            range,
            must_see,
        } => {
            let r = range.unwrap_or(m.attrs.follow_range).clamp(1.0, 128.0);
            let mut candidates: Vec<(EntityMemory, Who, f64)> = Vec::new();
            for s in w.nearby(m.position, r, |s| s.id != m.id && s.alive) {
                let ok = match types {
                    None => true,
                    Some(list) => seen_is_type(&list.species, &list.custom, s),
                };
                if ok {
                    let d2 = m.distance_sqr(s.position);
                    if d2 <= r * r {
                        candidates.push((EntityMemory::Mob(s.id), Who::Mob(s.id), d2));
                    }
                }
            }
            let allow_player = types.as_ref().is_none_or(|list| list.player);
            if allow_player && let Some(p) = &w.player && p.alive && !p.spectator {
                let d2 = m.distance_sqr(p.position);
                if d2 <= r * r {
                    candidates.push((EntityMemory::Player(p.uuid), Who::Player, d2));
                }
            }
            candidates.sort_by(|a, b| a.2.total_cmp(&b.2));
            let mut found = None;
            for (entity, who, _) in candidates {
                if !must_see || m.sees(w, who) {
                    found = Some(MemoryValue::Entity(entity));
                    break;
                }
            }
            write_optional(memories, &sensor.memory, sensor.ttl, found);
        }
        SensorKind::HurtBy { max_age } => {
            let Some((dmg, at)) = m.last_damage else {
                return;
            };
            let is_new = *last_damage_at != Some(at);
            let within_age = max_age.is_none_or(|limit| i64::from(m.age) - at <= limit);
            if !within_age {
                if sensor.ttl.is_none() {
                    memories.erase(&sensor.memory);
                }
                *last_damage_at = Some(at);
                return;
            }
            if !is_new && !(sensor.ttl.is_none() && max_age.is_some()) {
                return;
            }
            *last_damage_at = Some(at);
            if sensor.memory_kind == MemoryKind::Bool {
                memories.set(sensor.memory.clone(), MemoryValue::Bool(true), sensor.ttl);
                return;
            }
            let entity = match dmg.attacker {
                Some(Who::Mob(id)) => Some(EntityMemory::Mob(id)),
                Some(Who::Player) => Some(EntityMemory::Player(
                    w.player.as_ref().map_or([0; 4], |p| p.uuid),
                )),
                None => None,
            };
            if let Some(entity) = entity {
                memories.set(
                    sensor.memory.clone(),
                    MemoryValue::Entity(entity),
                    sensor.ttl,
                );
            }
        }
        SensorKind::Blocks {
            tag,
            blocks,
            radius,
            vertical_radius,
        } => {
            let here = [
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            ];
            let found = w.nearest_block(here, *radius, *vertical_radius, |_, name| {
                tag.as_deref().is_some_and(|t| w.has_tag(t, name))
                    || blocks.iter().any(|b| b == name)
            });
            let value = found.map(|p| match sensor.memory_kind {
                MemoryKind::Vec3 => {
                    MemoryValue::Vec3([p[0] as f32 + 0.5, p[1] as f32, p[2] as f32 + 0.5])
                }
                _ => MemoryValue::BlockPos(p),
            });
            write_optional(memories, &sensor.memory, sensor.ttl, value);
        }
        SensorKind::Light { kind } => {
            let x = m.position.x.floor() as i32;
            let y = m.position.y.floor() as i32;
            let z = m.position.z.floor() as i32;
            let packed = w.world.light(x, y, z);
            let sky = i32::from(packed >> 4);
            let block = i32::from(packed & 15);
            let level = match kind {
                LightKind::Combined => (sky - w.sky_darken()).max(block).clamp(0, 15),
                LightKind::Sky => sky.clamp(0, 15),
                LightKind::Block => block.clamp(0, 15),
            };
            let value = match sensor.memory_kind {
                MemoryKind::F32 => MemoryValue::F32(level as f32),
                MemoryKind::Bool => MemoryValue::Bool(level > 0),
                _ => MemoryValue::I32(level),
            };
            memories.set(sensor.memory.clone(), value, sensor.ttl);
        }
    }
}

fn write_optional(
    memories: &mut MemoryBank,
    key: &str,
    ttl: Option<i32>,
    value: Option<MemoryValue>,
) {
    match value {
        Some(v) => memories.set(key.to_owned(), v, ttl),
        // Without a TTL, an empty scan clears the memory so predicates react immediately;
        // with a TTL, let the remembered entry expire naturally after `ttl` ticks.
        None if ttl.is_none() => {
            memories.erase(key);
        }
        None => {}
    }
}
