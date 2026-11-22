//! Typed brain memories with optional expiry (`ai.memories`, F4M-4b).
//!
//! Each memory slot has a declared [`MemoryKind`], an optional initial default, an optional
//! default TTL in ticks, and a `persistent` flag (`yoake.memories` in NBT).
//!
//! Expiry follows Minecraft's `ExpirableValue` / `forgetOutdatedMemories` timing: a memory set
//! with `ttl = Some(N)` (`N >= 1`) stays present on the tick it is set and for `N - 1` tick
//! boundaries after, expiring at the start of tick `N`. `ttl = Some(0)` expires at the start of
//! the next tick; `ttl = None` never expires until overwritten or erased.
use horizon_mods::custom_mob::{MemoryDef as FileMemoryDef, limits};
use serde_json::Value;
use std::collections::BTreeMap;

/// The declared type of a brain memory slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    Bool,
    I32,
    F32,
    Str,
    Vec3,
    BlockPos,
    Entity,
}

impl MemoryKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "bool" => Some(Self::Bool),
            "i32" => Some(Self::I32),
            "f32" => Some(Self::F32),
            "string" => Some(Self::Str),
            "vec3" => Some(Self::Vec3),
            "block_pos" => Some(Self::BlockPos),
            "entity" => Some(Self::Entity),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::I32 => "i32",
            Self::F32 => "f32",
            Self::Str => "string",
            Self::Vec3 => "vec3",
            Self::BlockPos => "block_pos",
            Self::Entity => "entity",
        }
    }
}

/// An entity stored in an `entity` memory: a mob by runtime id, or a player by UUID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityMemory {
    Mob(u32),
    Player([i32; 4]),
}

/// A typed value held in a brain memory slot.
#[derive(Clone, Debug, PartialEq)]
pub enum MemoryValue {
    Bool(bool),
    I32(i32),
    F32(f32),
    Str(String),
    Vec3([f32; 3]),
    BlockPos([i32; 3]),
    Entity(EntityMemory),
}

impl MemoryValue {
    pub fn kind(&self) -> MemoryKind {
        match self {
            Self::Bool(_) => MemoryKind::Bool,
            Self::I32(_) => MemoryKind::I32,
            Self::F32(_) => MemoryKind::F32,
            Self::Str(_) => MemoryKind::Str,
            Self::Vec3(_) => MemoryKind::Vec3,
            Self::BlockPos(_) => MemoryKind::BlockPos,
            Self::Entity(_) => MemoryKind::Entity,
        }
    }
}

/// One memory definition of a custom mob type.
#[derive(Clone, Debug, PartialEq)]
pub struct MemoryDef {
    pub name: String,
    pub kind: MemoryKind,
    pub default: Option<MemoryValue>,
    pub persistent: bool,
    pub ttl: Option<i32>,
}

impl MemoryDef {
    pub fn from_file(name: &str, f: &FileMemoryDef) -> Result<Self, String> {
        let kind = MemoryKind::parse(&f.kind)
            .ok_or_else(|| format!("memory {name}: unknown type {:?}", f.kind))?;
        let default = match &f.default {
            None => None,
            Some(v) => Some(value_from_json(kind, v).ok_or_else(|| {
                format!("memory {name}: default {:?} does not fit {}", v, f.kind)
            })?),
        };
        let ttl = f
            .ttl
            .map(|t| i32::try_from(t.min(limits::MAX_MEMORY_TTL)).unwrap_or(i32::MAX));
        Ok(Self {
            name: name.to_owned(),
            kind,
            default,
            persistent: f.persistent,
            ttl,
        })
    }

    /// Validates and normalizes `v` for this memory slot (`None` if the type mismatches or a float
    /// is non-finite).
    pub fn coerce(&self, v: MemoryValue) -> Option<MemoryValue> {
        coerce_for_kind(self.kind, v)
    }
}

/// Validates and normalizes `v` for `kind`.
pub fn coerce_for_kind(kind: MemoryKind, v: MemoryValue) -> Option<MemoryValue> {
    match (kind, v) {
        (MemoryKind::Bool, v @ MemoryValue::Bool(_)) => Some(v),
        (MemoryKind::I32, v @ MemoryValue::I32(_)) => Some(v),
        (MemoryKind::F32, MemoryValue::F32(f)) if f.is_finite() => Some(MemoryValue::F32(f)),
        (MemoryKind::Str, MemoryValue::Str(s)) => {
            Some(MemoryValue::Str(truncate(&s, limits::MAX_MEMORY_STRING)))
        }
        (MemoryKind::Vec3, MemoryValue::Vec3(p)) if p.iter().all(|f| f.is_finite()) => {
            Some(MemoryValue::Vec3(p))
        }
        (MemoryKind::BlockPos, v @ MemoryValue::BlockPos(_)) => Some(v),
        (MemoryKind::Entity, v @ MemoryValue::Entity(_)) => Some(v),
        _ => None,
    }
}

/// Parses a JSON literal for `kind` (`entity` has no JSON literal).
pub fn value_from_json(kind: MemoryKind, v: &Value) -> Option<MemoryValue> {
    match kind {
        MemoryKind::Bool => v.as_bool().map(MemoryValue::Bool),
        MemoryKind::I32 => v.as_i64().and_then(|n| i32::try_from(n).ok()).map(MemoryValue::I32),
        MemoryKind::F32 => v.as_f64().filter(|f| f.is_finite()).map(|f| MemoryValue::F32(f as f32)),
        MemoryKind::Str => v
            .as_str()
            .filter(|s| s.len() <= limits::MAX_MEMORY_STRING)
            .map(|s| MemoryValue::Str(s.to_owned())),
        MemoryKind::Vec3 => {
            let a = v.as_array().filter(|a| a.len() == 3)?;
            let mut out = [0.0f32; 3];
            for (o, item) in out.iter_mut().zip(a) {
                *o = item.as_f64().filter(|f| f.is_finite())? as f32;
            }
            Some(MemoryValue::Vec3(out))
        }
        MemoryKind::BlockPos => {
            let a = v.as_array().filter(|a| a.len() == 3)?;
            let mut out = [0i32; 3];
            for (o, item) in out.iter_mut().zip(a) {
                *o = i32::try_from(item.as_i64()?).ok()?;
            }
            Some(MemoryValue::BlockPos(out))
        }
        MemoryKind::Entity => None,
    }
}

fn truncate(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

/// One live memory entry with optional remaining lifetime in ticks.
#[derive(Clone, Debug, PartialEq)]
pub struct MemoryEntry {
    pub value: MemoryValue,
    pub ttl: Option<i32>,
}

/// Per-mob memory table (`CustomState.brain.memories`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MemoryBank {
    entries: BTreeMap<String, MemoryEntry>,
}

impl MemoryBank {
    /// Initializes a bank with any declared `default` values (permanent).
    pub fn from_defs(defs: &[MemoryDef]) -> Self {
        let mut bank = Self::default();
        for def in defs {
            if let Some(default) = &def.default {
                bank.set(def.name.clone(), default.clone(), None);
            }
        }
        bank
    }

    pub fn get(&self, key: &str) -> Option<&MemoryValue> {
        self.entries.get(key).map(|e| &e.value)
    }

    pub fn entry(&self, key: &str) -> Option<&MemoryEntry> {
        self.entries.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &MemoryEntry)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn set(&mut self, key: String, value: MemoryValue, ttl: Option<i32>) {
        self.entries.insert(key, MemoryEntry { value, ttl });
    }

    pub fn erase(&mut self, key: &str) -> bool {
        self.entries.remove(key).is_some()
    }

    /// `Brain.forgetOutdatedMemories`: ticks down every expirable entry and removes expired ones.
    pub fn forget_outdated(&mut self) {
        self.entries.retain(|_, entry| match &mut entry.ttl {
            None => true,
            Some(ttl) => {
                *ttl -= 1;
                *ttl > 0
            }
        });
    }
}
