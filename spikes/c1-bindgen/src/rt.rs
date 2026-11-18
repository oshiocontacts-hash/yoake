pub mod codec;

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    UnexpectedEnd,
    TrailingBytes,
    InvalidBool(u8),
    InvalidTag(&'static str, u8),
    InvalidUtf8,
    LengthOverflow,
    UnknownHandle,
    StaleHandle,
    BorrowConflict,
    InvalidPath(String),
}

impl fmt::Display for CallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEnd => write!(formatter, "argument buffer ended early"),
            Self::TrailingBytes => write!(formatter, "argument buffer has trailing bytes"),
            Self::InvalidBool(tag) => write!(formatter, "invalid bool tag {tag}"),
            Self::InvalidTag(kind, tag) => write!(formatter, "invalid {kind} tag {tag}"),
            Self::InvalidUtf8 => write!(formatter, "string is not UTF-8"),
            Self::LengthOverflow => write!(formatter, "length does not fit this host"),
            Self::UnknownHandle => write!(formatter, "unknown or released handle"),
            Self::StaleHandle => write!(formatter, "stale handle generation"),
            Self::BorrowConflict => write!(formatter, "path borrow conflicts with an active borrow"),
            Self::InvalidPath(path) => write!(formatter, "invalid path: {path}"),
        }
    }
}

impl Error for CallError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Root {
    Game,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum Accessor {
    Field(String),
    Entity(u64),
    Slot(usize),
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PathHandle {
    pub root: Root,
    pub accessors: Vec<Accessor>,
}

impl PathHandle {
    pub fn game() -> Self {
        Self {
            root: Root::Game,
            accessors: Vec::new(),
        }
    }

    pub fn field(mut self, name: impl Into<String>) -> Self {
        self.accessors.push(Accessor::Field(name.into()));
        self
    }

    pub fn entity(mut self, id: u64) -> Self {
        self.accessors.push(Accessor::Entity(id));
        self
    }

    pub fn slot(mut self, slot: usize) -> Self {
        self.accessors.push(Accessor::Slot(slot));
        self
    }

    pub fn overlaps(&self, other: &Self) -> bool {
        self.root == other.root
            && (self.accessors.starts_with(&other.accessors)
                || other.accessors.starts_with(&self.accessors))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Handle {
    slot: u32,
    generation: u32,
}

impl Handle {
    pub fn to_bits(self) -> u64 {
        (u64::from(self.generation) << 32) | u64::from(self.slot)
    }

    pub fn from_bits(bits: u64) -> Self {
        Self {
            slot: bits as u32,
            generation: (bits >> 32) as u32,
        }
    }
}

#[derive(Default)]
struct HandleSlot {
    generation: u32,
    path: Option<PathHandle>,
}

#[derive(Default)]
pub struct HandleTable {
    slots: Vec<HandleSlot>,
    free: Vec<u32>,
}

impl HandleTable {
    pub fn insert(&mut self, path: PathHandle) -> Handle {
        if let Some(index) = self.free.pop() {
            let slot = &mut self.slots[index as usize];
            debug_assert!(slot.path.is_none());
            slot.path = Some(path);
            return Handle {
                slot: index,
                generation: slot.generation,
            };
        }
        let index = self.slots.len() as u32;
        self.slots.push(HandleSlot {
            generation: 1,
            path: Some(path),
        });
        Handle {
            slot: index,
            generation: 1,
        }
    }

    pub fn resolve(&self, handle: Handle) -> Result<&PathHandle, CallError> {
        let slot = self.slots.get(handle.slot as usize).ok_or(CallError::UnknownHandle)?;
        if slot.generation != handle.generation {
            return Err(CallError::StaleHandle);
        }
        slot.path.as_ref().ok_or(CallError::UnknownHandle)
    }

    pub fn remove(&mut self, handle: Handle) -> Result<PathHandle, CallError> {
        let slot = self.slots.get_mut(handle.slot as usize).ok_or(CallError::UnknownHandle)?;
        if slot.generation != handle.generation {
            return Err(CallError::StaleHandle);
        }
        let path = slot.path.take().ok_or(CallError::UnknownHandle)?;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        self.free.push(handle.slot);
        Ok(path)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BorrowMode {
    Shared,
    Exclusive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BorrowToken(u64);

struct Grant {
    path: PathHandle,
    mode: BorrowMode,
}

#[derive(Default)]
pub struct BorrowTable {
    next_token: u64,
    grants: BTreeMap<u64, Grant>,
}

impl BorrowTable {
    pub fn acquire(
        &mut self,
        path: &PathHandle,
        mode: BorrowMode,
    ) -> Result<BorrowToken, CallError> {
        let conflict = self.grants.values().any(|grant| {
            path.overlaps(&grant.path)
                && (mode == BorrowMode::Exclusive || grant.mode == BorrowMode::Exclusive)
        });
        if conflict {
            return Err(CallError::BorrowConflict);
        }
        self.next_token = self.next_token.wrapping_add(1).max(1);
        let token = BorrowToken(self.next_token);
        self.grants.insert(
            token.0,
            Grant {
                path: path.clone(),
                mode,
            },
        );
        Ok(token)
    }

    pub fn release(&mut self, token: BorrowToken) -> Result<(), CallError> {
        self.grants
            .remove(&token.0)
            .map(|_| ())
            .ok_or(CallError::UnknownHandle)
    }
}

pub trait ResolvePath<T> {
    fn resolve_path<'a>(&'a mut self, path: &PathHandle) -> Result<&'a mut T, CallError>;
}
