//! Typed native-mod data attached to server-owned holders.
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::{Arc, Mutex},
};

use crate::nbt::{Compound, JavaString, Tag};

/// Copy policy of every static (native) attachment type seen in this process,
/// by namespace and name. Filled by `register_static`, which every use of a
/// type calls, and which a native mod calls at startup for the types it
/// declares so a type nothing has touched yet is still known at respawn.
static STATIC_TYPES: Mutex<BTreeMap<(String, String), bool>> = Mutex::new(BTreeMap::new());

fn static_policy(namespace: &str, name: &str) -> Option<bool> {
    STATIC_TYPES
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&(namespace.to_owned(), name.to_owned()))
        .copied()
}

/// Record a static type's `copy_on_death` so values saved under it are judged
/// by it before anything reads them. Registering the same type again is a
/// no-op; a second type with the same `namespace:name` and another policy is a
/// conflict and panics.
pub fn register_static<T>(ty: &AttachmentType<T>) {
    let mut types = STATIC_TYPES.lock().unwrap_or_else(|error| error.into_inner());
    let key = (ty.namespace.to_owned(), ty.name.to_owned());
    match types.get(&key) {
        Some(&known) if known != ty.copy_on_death => panic!(
            "conflicting attachment types for {}:{}: copy_on_death is both {known} and {}",
            ty.namespace, ty.name, ty.copy_on_death
        ),
        Some(_) => {}
        None => {
            types.insert(key, ty.copy_on_death);
        }
    }
}

/// An attachment operation could not be completed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttachmentError {
    /// The holder does not exist or is unavailable on this side.
    MissingHolder,
    /// The key was used with a different Rust type.
    TypeMismatch,
    /// The saved value could not be decoded.
    Decode,
    /// The value could not be encoded.
    Encode,
    /// A namespace in the saved compound is not a compound.
    InvalidNamespace,
    /// Copy-on-death requires a serializer.
    InvalidCopyPolicy,
    /// Syncing to clients requires a serializer.
    InvalidSyncPolicy,
    /// The identifier is not a valid namespaced identifier.
    InvalidId,
    /// A mod attempted to define another mod's identifier.
    WrongOwner,
    /// A definition already exists for this identifier.
    DuplicateType,
    /// The holder does not match the declared scope.
    ScopeMismatch,
    /// This package's world-wide attachment budget would be exceeded.
    QuotaExceeded,
}

/// NBT key reserved for server-owned attachment maps.
pub const ATTACHMENTS_KEY: &str = "horizon:attachments";
/// Maximum length of a dynamic attachment identifier in UTF-8 bytes.
pub const MAX_ATTACHMENT_ID_BYTES: usize = 128;
/// Default per-package, per-world attachment byte budget.
pub const DEFAULT_ATTACHMENT_BUDGET: usize = 32 * 1024 * 1024;

type Usage = BTreeMap<JavaString, (usize, usize)>;

#[derive(Default)]
struct BudgetState {
    generation: u64,
    holders: BTreeMap<String, (u64, Usage)>,
    totals: BTreeMap<JavaString, usize>,
    scan_failed: bool,
}

struct BudgetBinding {
    state: Arc<Mutex<BudgetState>>,
    holder: String,
    generation: u64,
    limit: usize,
}

impl BudgetBinding {
    fn update(&self, usage: Usage, enforce: bool) -> Result<(), AttachmentError> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let Some((generation, previous)) = state.holders.get(&self.holder) else {
            return Err(AttachmentError::MissingHolder);
        };
        if *generation != self.generation {
            return Err(AttachmentError::MissingHolder);
        }
        let previous = previous.clone();
        if enforce {
            for (namespace, (bytes, _)) in &usage {
                let old = previous.get(namespace).map_or(0, |value| value.0);
                let total = state.totals.get(namespace).copied().unwrap_or(0);
                if *bytes > old
                    && (state.scan_failed
                        || total.saturating_sub(old).saturating_add(*bytes) > self.limit)
                {
                    return Err(AttachmentError::QuotaExceeded);
                }
            }
        }
        for (namespace, (bytes, _)) in previous {
            let total = state.totals.entry(namespace).or_default();
            *total = total.saturating_sub(bytes);
        }
        for (namespace, (bytes, _)) in &usage {
            let total = state.totals.entry(namespace.clone()).or_default();
            *total = total.saturating_add(*bytes);
        }
        state
            .holders
            .insert(self.holder.clone(), (self.generation, usage));
        state.totals.retain(|_, bytes| *bytes != 0);
        Ok(())
    }
}

impl Drop for BudgetBinding {
    fn drop(&mut self) {
        let usage = {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let Some((generation, usage)) = state.holders.get(&self.holder) else {
                return;
            };
            if *generation != self.generation {
                return;
            }
            usage
                .iter()
                .filter_map(|(namespace, (_, persistent))| {
                    (*persistent != 0).then(|| (namespace.clone(), (*persistent, *persistent)))
                })
                .collect()
        };
        let _ = self.update(usage, false);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state
            .holders
            .get(&self.holder)
            .is_some_and(|(generation, usage)| *generation == self.generation && usage.is_empty())
        {
            state.holders.remove(&self.holder);
        }
    }
}

/// Maximum encoded size of a synced value, reserving room for its key and holder.
pub const MAX_SYNCED_ATTACHMENT_BYTES: usize = 1_048_576 - 1_600;

fn validate_synced_value(
    namespace: &str,
    name: &str,
    value: &Compound,
) -> Result<(), AttachmentError> {
    let limits = crate::nbt::Limits {
        max_bytes: MAX_SYNCED_ATTACHMENT_BYTES,
        ..crate::nbt::Limits::default()
    };
    if namespace.encode_utf16().count() > 256
        || name.encode_utf16().count() > 256
        || crate::nbt::network_compound_encoded_size(value, limits).is_err()
    {
        log::warn!("Refusing an attachment value that cannot be synced");
        return Err(AttachmentError::Encode);
    }
    Ok(())
}

fn encodable_name(name: &JavaString) -> bool {
    let mut length = 0usize;
    for unit in &name.0 {
        length += match unit {
            0 => 2,
            1..=0x7f => 1,
            0x80..=0x7ff => 2,
            _ => 3,
        };
        if length > u16::MAX as usize {
            return false;
        }
    }
    true
}

fn encodable_entry(name: &JavaString, value: &Tag) -> bool {
    let limits = crate::nbt::Limits {
        max_depth: crate::nbt::Limits::saved().max_depth - 3,
        ..crate::nbt::Limits::saved()
    };
    let valid = encodable_name(name)
        && !matches!(value, Tag::End)
        && crate::nbt::network_encoded_size(value, limits).is_ok();
    if !valid {
        use std::sync::atomic::{AtomicBool, Ordering};
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::Relaxed) {
            log::warn!("Skipping an attachment that cannot be encoded in a save");
        }
    }
    valid
}

/// Read the reserved compound without replacing an invalid saved value.
pub fn attachment_compound(tag: Option<&Tag>) -> Result<Option<&Compound>, AttachmentError> {
    match tag {
        None => Ok(None),
        Some(Tag::Compound(compound)) => Ok(Some(compound)),
        Some(_) => Err(AttachmentError::InvalidNamespace),
    }
}

/// A server holder in the host's current dimension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Holder {
    Entity(u32),
    Player(u128),
    BlockEntity([i32; 3]),
    Chunk([i32; 2]),
    Level,
}

/// Object-safe access to the current server's holders.
pub trait AttachmentHost {
    fn attachments(&mut self, holder: Holder) -> Result<&mut Attachments, AttachmentError>;
}

/// Holder kinds supported by dynamically declared attachments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentScope {
    Player,
    Entity,
    BlockEntity,
    Chunk,
    Level,
}

impl AttachmentScope {
    fn matches(self, holder: Holder) -> bool {
        matches!(
            (self, holder),
            (Self::Player, Holder::Player(_))
                | (Self::Entity, Holder::Entity(_))
                | (Self::BlockEntity, Holder::BlockEntity(_))
                | (Self::Chunk, Holder::Chunk(_))
                | (Self::Level, Holder::Level)
        )
    }
}

/// Storage and delivery policy for a dynamic attachment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DynamicAttachmentPolicy {
    pub scope: AttachmentScope,
    pub persistent: bool,
    pub synced: bool,
    pub copy_on_death: bool,
}

/// An NBT attachment definition whose identifier is known only at runtime.
#[derive(Clone, Debug)]
pub struct DynamicAttachmentType {
    namespace: String,
    name: String,
    scope: AttachmentScope,
    persistent: bool,
    synced: bool,
    copy_on_death: bool,
}

/// Definitions are unique within a registry and confined to their owner's namespace.
#[derive(Clone, Default)]
pub struct DynamicAttachmentRegistry {
    types: BTreeMap<(String, String), DynamicAttachmentType>,
    budget: Arc<Mutex<BudgetState>>,
}

impl DynamicAttachmentRegistry {
    /// Whether a package has registered attachments in this world.
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// Refuse growth when the world's saved usage could not be completely indexed.
    pub fn deny_growth(&self) {
        self.budget
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .scan_failed = true;
    }

    /// Account for an unloaded saved holder before any package can allocate more data.
    pub fn account_saved(&self, holder: String, raw: Compound) {
        let mut attachments = Attachments::from_nbt(raw);
        self.bind(&mut attachments, holder);
    }

    /// Register an identifier for the trusted mod owner supplied by the host.
    pub fn define(
        &mut self,
        owner: &str,
        id: &str,
        policy: DynamicAttachmentPolicy,
    ) -> Result<DynamicAttachmentType, AttachmentError> {
        if id.len() > MAX_ATTACHMENT_ID_BYTES {
            return Err(AttachmentError::InvalidId);
        }
        let (namespace, name) = id.split_once(':').ok_or(AttachmentError::InvalidId)?;
        let valid = |byte: u8| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.-".contains(&byte)
        };
        if namespace.is_empty()
            || !namespace.bytes().all(valid)
            || name.is_empty()
            || !name.bytes().all(|byte| valid(byte) || byte == b'/')
            || name
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(AttachmentError::InvalidId);
        }
        if namespace != owner {
            return Err(AttachmentError::WrongOwner);
        }
        let key = (namespace.to_owned(), name.to_owned());
        if self.types.contains_key(&key) {
            return Err(AttachmentError::DuplicateType);
        }
        if let Some(known) = static_policy(namespace, name) {
            assert_eq!(
                known, policy.copy_on_death,
                "conflicting attachment types for {id}: a static type already declares copy_on_death {known}"
            );
        }
        let definition = DynamicAttachmentType {
            namespace: key.0.clone(),
            name: key.1.clone(),
            scope: policy.scope,
            persistent: policy.persistent,
            synced: policy.synced,
            copy_on_death: policy.copy_on_death,
        };
        self.types.insert(key, definition.clone());
        Ok(definition)
    }

    /// Look up one of the owner's definitions by its namespaced identifier.
    pub fn get(&self, owner: &str, id: &str) -> Option<&DynamicAttachmentType> {
        let (namespace, name) = id.split_once(':')?;
        if namespace != owner {
            return None;
        }
        self.types.get(&(namespace.to_owned(), name.to_owned()))
    }

    /// Apply current policies to saved values before exposing or copying a holder.
    pub fn reconcile<H: AttachmentHost + ?Sized>(
        &self,
        host: &mut H,
        holder: Holder,
    ) -> Result<(), AttachmentError> {
        let attachments = host.attachments(holder)?;
        self.reconcile_map(holder, attachments)
    }

    /// Respawn copy: resolve every saved value whose type is defined here, so
    /// its own `copy_on_death` decides (even if nothing read it yet), then copy.
    /// Values of unknown types are carried over raw.
    pub fn copy_on_death(
        &self,
        holder: Holder,
        attachments: &mut Attachments,
    ) -> Result<Attachments, AttachmentError> {
        for ((namespace, name), definition) in &self.types {
            if let Some(known) = static_policy(namespace, name) {
                assert_eq!(
                    known, definition.copy_on_death,
                    "conflicting attachment types for {namespace}:{name}: a static type declares copy_on_death {known}"
                );
            }
        }
        self.reconcile_map(holder, attachments)?;
        attachments.copy_on_death()
    }

    /// Join a holder to this world's shared budget, retaining unloaded saved usage.
    pub fn bind(&self, attachments: &mut Attachments, holder: String) {
        if attachments.budget.as_ref().is_some_and(|binding| {
            Arc::ptr_eq(&binding.state, &self.budget)
                && binding.holder == holder
                && self
                    .budget
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .holders
                    .get(&holder)
                    .is_some_and(|(generation, _)| *generation == binding.generation)
        }) {
            return;
        }
        let mut state = self
            .budget
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.generation += 1;
        let generation = state.generation;
        let previous = state
            .holders
            .get(&holder)
            .map(|(_, usage)| usage.clone())
            .unwrap_or_default();
        state.holders.insert(holder.clone(), (generation, previous));
        drop(state);
        attachments.budget = Some(Arc::new(BudgetBinding {
            state: self.budget.clone(),
            holder,
            generation,
            limit: DEFAULT_ATTACHMENT_BUDGET,
        }));
        let _ = attachments.refresh_budget(false);
    }

    /// Reconcile an already-resolved holder without resolving or dirtying it again.
    pub fn reconcile_map(
        &self,
        holder: Holder,
        attachments: &mut Attachments,
    ) -> Result<(), AttachmentError> {
        let mut result = Ok(());
        attachments.entries.retain(|(namespace, name), entry| {
            let Entry::Dynamic(value) = entry else {
                return true;
            };
            if self
                .types
                .get(&(namespace.to_string_lossy(), name.to_string_lossy()))
                .is_some_and(|definition| definition.scope.matches(holder))
            {
                return true;
            }
            if value.persistent {
                *entry = Entry::Raw(value.envelope());
                true
            } else {
                false
            }
        });
        for definition in self
            .types
            .values()
            .filter(|definition| definition.scope.matches(holder))
        {
            if let Err(error) = attachments.get_dynamic(definition) {
                result = Err(error);
            }
        }
        attachments.refresh_budget(false)?;
        result
    }
}

impl DynamicAttachmentType {
    /// The storage and delivery policy this type was defined with.
    pub fn policy(&self) -> DynamicAttachmentPolicy {
        DynamicAttachmentPolicy {
            scope: self.scope,
            persistent: self.persistent,
            synced: self.synced,
            copy_on_death: self.copy_on_death,
        }
    }

    fn key(&self) -> (JavaString, JavaString) {
        (self.namespace.as_str().into(), self.name.as_str().into())
    }

    fn holder<'a, H: AttachmentHost + ?Sized>(
        &self,
        host: &'a mut H,
        holder: Holder,
    ) -> Result<&'a mut Attachments, AttachmentError> {
        if !self.scope.matches(holder) {
            return Err(AttachmentError::ScopeMismatch);
        }
        host.attachments(holder)
    }

    /// Read an existing NBT value without inserting a default.
    pub fn get<H: AttachmentHost + ?Sized>(
        &self,
        host: &mut H,
        holder: Holder,
    ) -> Result<Option<Compound>, AttachmentError> {
        self.holder(host, holder)?.get_dynamic(self)
    }

    /// Read a value without materializing it or changing the holder.
    pub fn read(
        &self,
        holder: Holder,
        attachments: Option<&Attachments>,
    ) -> Result<Option<Compound>, AttachmentError> {
        if !self.scope.matches(holder) {
            return Err(AttachmentError::ScopeMismatch);
        }
        match attachments {
            Some(attachments) => attachments.read_dynamic(self),
            None => Ok(None),
        }
    }

    /// Replace an NBT value and queue it for sync if requested.
    pub fn set<H: AttachmentHost + ?Sized>(
        &self,
        host: &mut H,
        holder: Holder,
        value: Compound,
    ) -> Result<(), AttachmentError> {
        self.holder(host, holder)?.set_dynamic(self, value)
    }

    /// Remove an existing NBT value and queue a synced removal if requested.
    pub fn remove<H: AttachmentHost + ?Sized>(
        &self,
        host: &mut H,
        holder: Holder,
    ) -> Result<Option<Compound>, AttachmentError> {
        self.holder(host, holder)?.remove_dynamic(self)
    }
}

/// The optional on-disk representation of an attachment.
pub struct AttachmentCodec<T> {
    pub encode: fn(&T) -> Result<Compound, AttachmentError>,
    pub decode: fn(&Compound) -> Result<T, AttachmentError>,
}

/// A mod-defined attachment key and its value's behavior.
pub struct AttachmentType<T: 'static> {
    pub namespace: &'static str,
    pub name: &'static str,
    pub default: fn() -> T,
    pub codec: Option<AttachmentCodec<T>>,
    pub copy_on_death: bool,
    /// Send the value to the clients that see the holder; needs a codec.
    pub sync: bool,
}

impl<T> AttachmentType<T> {
    fn validate_policy(&self) -> Result<(), AttachmentError> {
        if self.copy_on_death && self.codec.is_none() {
            return Err(AttachmentError::InvalidCopyPolicy);
        }
        if self.sync && self.codec.is_none() {
            return Err(AttachmentError::InvalidSyncPolicy);
        }
        register_static(self);
        Ok(())
    }
}

/// One attachment's new state for a client; `None` means removed.
#[derive(Clone, Debug, PartialEq)]
pub struct AttachmentSync {
    pub namespace: JavaString,
    pub name: JavaString,
    pub value: Option<Compound>,
}

impl<T: Clone + Send + Sync + 'static> AttachmentType<T> {
    /// Read a value, inserting its default if absent; see `get_existing`.
    pub fn get<H: AttachmentHost + ?Sized>(
        &self,
        host: &mut H,
        holder: Holder,
    ) -> Result<T, AttachmentError> {
        host.attachments(holder)?.get(self)
    }
    /// Whether a holder carries a value, like NeoForge's `hasData`; never inserts the default.
    /// API 317.
    pub fn has<H: AttachmentHost + ?Sized>(
        &self,
        host: &mut H,
        holder: Holder,
    ) -> Result<bool, AttachmentError> {
        Ok(host.attachments(holder)?.has(self))
    }
    /// Read a value only if present, like `getExistingData`. API 317.
    pub fn get_existing<H: AttachmentHost + ?Sized>(
        &self,
        host: &mut H,
        holder: Holder,
    ) -> Result<Option<T>, AttachmentError> {
        host.attachments(holder)?.get_existing(self)
    }
    /// Replace a value on an existing holder.
    pub fn set<H: AttachmentHost + ?Sized>(
        &self,
        host: &mut H,
        holder: Holder,
        value: T,
    ) -> Result<(), AttachmentError> {
        host.attachments(holder)?.set(self, value)
    }
    /// Remove a value from an existing holder.
    pub fn remove<H: AttachmentHost + ?Sized>(
        &self,
        host: &mut H,
        holder: Holder,
    ) -> Result<Option<T>, AttachmentError> {
        host.attachments(holder)?.remove(self)
    }
    /// Send the current value to the clients that see the holder.
    pub fn sync_data<H: AttachmentHost + ?Sized>(
        &self,
        host: &mut H,
        holder: Holder,
    ) -> Result<(), AttachmentError> {
        host.attachments(holder)?.mark_sync(self)
    }
}

trait Value: Send + Sync {
    fn as_any(&self) -> &dyn Any;
    fn clone_box(&self) -> Box<dyn Value>;
    fn encoded(&self) -> Option<&Compound>;
    fn copy_on_death(&self) -> bool;
    fn sync(&self) -> bool;
}
struct TypedValue<T> {
    value: T,
    encoded: Option<Compound>,
    copy_on_death: bool,
    sync: bool,
}
impl<T: Clone + Send + Sync + 'static> Value for TypedValue<T> {
    fn as_any(&self) -> &dyn Any {
        &self.value
    }
    fn clone_box(&self) -> Box<dyn Value> {
        Box::new(Self {
            value: self.value.clone(),
            encoded: self.encoded.clone(),
            copy_on_death: self.copy_on_death,
            sync: self.sync,
        })
    }
    fn encoded(&self) -> Option<&Compound> {
        self.encoded.as_ref()
    }
    fn copy_on_death(&self) -> bool {
        self.copy_on_death
    }
    fn sync(&self) -> bool {
        self.sync
    }
}
/// The saved form of a decoded value: its encoding and the policies it was
/// declared with. `None` when the type has no codec.
fn envelope(value: &dyn Value) -> Option<Compound> {
    value.encoded().map(|encoded| {
        let mut envelope = Compound::new();
        envelope.insert("value".into(), Tag::Compound(encoded.clone()));
        if value.copy_on_death() {
            envelope.insert("copy_on_death".into(), Tag::Byte(1));
        }
        if value.sync() {
            envelope.insert("sync".into(), Tag::Byte(1));
        }
        envelope
    })
}
fn raw_value(raw: &Compound) -> Option<&Compound> {
    match raw.get(&"value".into()) {
        Some(Tag::Compound(value)) => Some(value),
        _ => None,
    }
}

#[derive(Clone)]
struct DynamicValue {
    value: Compound,
    persistent: bool,
    synced: bool,
    copy_on_death: bool,
}

impl DynamicValue {
    fn envelope(&self) -> Compound {
        let mut result = Compound::new();
        result.insert("value".into(), Tag::Compound(self.value.clone()));
        if self.copy_on_death {
            result.insert("copy_on_death".into(), Tag::Byte(1));
        }
        if self.synced {
            result.insert("sync".into(), Tag::Byte(1));
        }
        result
    }
}

enum Entry {
    Raw(Compound),
    Decoded(Box<dyn Value>),
    Dynamic(DynamicValue),
}
impl Entry {
    /// The encoded value, as clients receive it.
    fn synced_value(&self) -> Option<&Compound> {
        match self {
            Self::Raw(raw) => raw_value(raw),
            Self::Decoded(value) => value.encoded(),
            Self::Dynamic(value) => Some(&value.value),
        }
    }
    fn is_synced(&self) -> bool {
        match self {
            Self::Raw(_) => false,
            Self::Decoded(value) => value.sync(),
            Self::Dynamic(value) => value.synced,
        }
    }
}
impl Clone for Entry {
    fn clone(&self) -> Self {
        match self {
            Self::Raw(value) => Self::Raw(value.clone()),
            Self::Decoded(value) => Self::Decoded(value.clone_box()),
            Self::Dynamic(value) => Self::Dynamic(value.clone()),
        }
    }
}

/// The data held by one entity, block entity, chunk or level.
#[derive(Default)]
pub struct Attachments {
    entries: BTreeMap<(JavaString, JavaString), Entry>,
    // Keep malformed or otherwise unrecognized NBT exactly as it was read.
    raw: Compound,
    // Keys to send to clients at the next flush; never saved.
    pending: BTreeSet<(JavaString, JavaString)>,
    budget: Option<Arc<BudgetBinding>>,
}
impl Clone for Attachments {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            raw: self.raw.clone(),
            pending: self.pending.clone(),
            budget: None,
        }
    }
}
impl fmt::Debug for Attachments {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Attachments")
            .field("keys", &self.entries.keys())
            .finish()
    }
}

impl Attachments {
    fn usage(&self, replacement: Option<(&(JavaString, JavaString), &Entry)>) -> Usage {
        let mut usage = Usage::new();
        let limits = crate::nbt::Limits {
            max_bytes: usize::MAX,
            ..crate::nbt::Limits::saved()
        };
        let mut count = |key: &(JavaString, JavaString), entry: &Entry| {
            let (encoded, persistent) = match entry {
                Entry::Raw(value) => (Some(value), true),
                Entry::Decoded(value) => (value.encoded(), value.encoded().is_some()),
                Entry::Dynamic(value) => (Some(&value.value), value.persistent),
            };
            let bytes = encoded
                .map_or_else(
                    || DEFAULT_ATTACHMENT_BUDGET + 1,
                    |value| {
                        crate::nbt::network_compound_encoded_size(value, limits)
                            .unwrap_or(DEFAULT_ATTACHMENT_BUDGET + 1)
                    },
                )
                .saturating_add(3 * (key.0.0.len() + key.1.0.len()) + 64);
            let total = usage.entry(key.0.clone()).or_default();
            total.0 = total.0.saturating_add(bytes);
            if persistent {
                total.1 = total.1.saturating_add(bytes);
            }
        };
        for (key, entry) in &self.entries {
            if replacement.is_none_or(|(replace, _)| replace != key) {
                count(key, entry);
            }
        }
        if let Some((key, entry)) = replacement {
            count(key, entry);
        }
        for (namespace, tag) in &self.raw {
            let mut count_raw = |name: Option<&JavaString>, tag: &Tag| {
                let bytes = crate::nbt::network_encoded_size(tag, limits)
                    .unwrap_or(DEFAULT_ATTACHMENT_BUDGET + 1)
                    .saturating_add(
                        3 * (namespace.0.len() + name.map_or(0, |name| name.0.len())) + 64,
                    );
                let total = usage.entry(namespace.clone()).or_default();
                total.0 = total.0.saturating_add(bytes);
                total.1 = total.1.saturating_add(bytes);
            };
            if let Tag::Compound(names) = tag {
                for (name, value) in names {
                    let key = (namespace.clone(), name.clone());
                    if !self.entries.contains_key(&key)
                        && replacement.is_none_or(|(replace, _)| replace != &key)
                    {
                        count_raw(Some(name), value);
                    }
                }
            } else {
                count_raw(None, tag);
            }
        }
        usage
    }

    fn reserve(
        &self,
        key: &(JavaString, JavaString),
        entry: &Entry,
    ) -> Result<(), AttachmentError> {
        if let Some(binding) = &self.budget {
            binding.update(self.usage(Some((key, entry))), true)?;
        }
        Ok(())
    }

    fn refresh_budget(&self, enforce: bool) -> Result<(), AttachmentError> {
        if let Some(binding) = &self.budget {
            binding.update(self.usage(None), enforce)?;
        }
        Ok(())
    }

    /// Read the namespaced attachment compound, leaving unknown data intact.
    pub fn from_nbt(raw: Compound) -> Self {
        let mut result = Self {
            entries: BTreeMap::new(),
            raw,
            pending: BTreeSet::new(),
            budget: None,
        };
        for (namespace, tag) in &result.raw {
            if let Tag::Compound(names) = tag {
                for (name, value) in names {
                    if let Tag::Compound(value) = value {
                        result
                            .entries
                            .insert((namespace.clone(), name.clone()), Entry::Raw(value.clone()));
                    }
                }
            }
        }
        result
    }
    /// Encode changed values, preserving raw entries and unrecognized tags.
    pub fn to_nbt(&self) -> Compound {
        let mut raw = self.raw.clone();
        for ((namespace, name), entry) in &self.entries {
            let value = match entry {
                Entry::Raw(value) => Some(value.clone()),
                Entry::Decoded(value) => envelope(value.as_ref()),
                Entry::Dynamic(value) => value.persistent.then(|| value.envelope()),
            };
            if let Some(value) = value {
                let tag = raw
                    .entry(namespace.clone())
                    .or_insert_with(|| Tag::Compound(Compound::new()));
                let Tag::Compound(names) = tag else {
                    unreachable!("attachment namespace checked before mutation");
                };
                names.insert(name.clone(), Tag::Compound(value));
            }
        }
        raw.retain(|namespace, tag| {
            if !encodable_name(namespace) {
                return encodable_entry(namespace, tag);
            }
            if let Tag::Compound(names) = tag {
                names.retain(|name, value| encodable_entry(name, value));
                true
            } else {
                encodable_entry(namespace, tag)
            }
        });
        raw
    }
    /// Order for an unread saved value: dynamic registry (see
    /// `DynamicAttachmentRegistry::copy_on_death`), then the static registry,
    /// then carried raw. Copy values for a respawn. Values whose current type is known are copied
    /// only when it opts in (never by a saved flag alone); a dynamic type is
    /// known once `DynamicAttachmentRegistry::reconcile_map` has run, which
    /// `DynamicAttachmentRegistry::copy_on_death` does first. A value that is
    /// still raw has no known type (a mod that is not loaded, or a native type
    /// not read yet), so it is carried over uninterpreted: it is never decoded
    /// or typed from its saved flags, and it is not silently dropped.
    pub fn copy_on_death(&self) -> Result<Self, AttachmentError> {
        let mut copy = Self::default();
        for (key, entry) in &self.entries {
            if let Entry::Dynamic(value) = entry {
                if value.copy_on_death {
                    copy.entries
                        .insert(key.clone(), Entry::Dynamic(value.clone()));
                }
                continue;
            }
            let copied = match entry {
                // Dynamic types were resolved by the registry before this; a
                // registered static type decides, and an unknown one is carried raw.
                Entry::Raw(raw) => static_policy(&key.0.to_string_lossy(), &key.1.to_string_lossy())
                    .unwrap_or(true)
                    .then(|| Entry::Raw(raw.clone())),
                Entry::Decoded(value) if value.copy_on_death() => {
                    envelope(value.as_ref()).ok_or(AttachmentError::InvalidCopyPolicy)?;
                    Some(Entry::Decoded(value.clone_box()))
                }
                _ => None,
            };
            if let Some(copied) = copied {
                copy.entries.insert(key.clone(), copied);
            }
        }
        Ok(copy)
    }

    fn get_dynamic(
        &mut self,
        ty: &DynamicAttachmentType,
    ) -> Result<Option<Compound>, AttachmentError> {
        let value = self.read_dynamic(ty)?;
        if let Some(value) = &value {
            self.insert_dynamic(ty, value.clone());
            self.refresh_budget(false)?;
        }
        Ok(value)
    }

    fn read_dynamic(
        &self,
        ty: &DynamicAttachmentType,
    ) -> Result<Option<Compound>, AttachmentError> {
        let key = ty.key();
        if self
            .raw
            .get(&key.0)
            .is_some_and(|tag| !matches!(tag, Tag::Compound(_)))
        {
            return Err(AttachmentError::InvalidNamespace);
        }
        let value = match self.entries.get(&key) {
            Some(Entry::Raw(raw)) => raw_value(raw).ok_or(AttachmentError::Decode)?.clone(),
            Some(Entry::Decoded(_)) => return Err(AttachmentError::TypeMismatch),
            Some(Entry::Dynamic(value)) => value.value.clone(),
            None => {
                if self.raw.get(&key.0).is_some_and(
                    |tag| matches!(tag, Tag::Compound(names) if names.contains_key(&key.1)),
                ) {
                    return Err(AttachmentError::Decode);
                }
                return Ok(None);
            }
        };
        Ok(Some(value))
    }

    fn insert_dynamic(&mut self, ty: &DynamicAttachmentType, value: Compound) {
        let key = ty.key();
        self.entries.insert(
            key.clone(),
            Entry::Dynamic(DynamicValue {
                value,
                persistent: ty.persistent,
                synced: ty.synced,
                copy_on_death: ty.copy_on_death,
            }),
        );
        if let Some(Tag::Compound(names)) = self.raw.get_mut(&key.0) {
            names.remove(&key.1);
        }
    }

    fn set_dynamic(
        &mut self,
        ty: &DynamicAttachmentType,
        value: Compound,
    ) -> Result<(), AttachmentError> {
        if ty.synced {
            validate_synced_value(&ty.namespace, &ty.name, &value)?;
        }
        self.get_dynamic(ty)?;
        self.reserve(
            &ty.key(),
            &Entry::Dynamic(DynamicValue {
                value: value.clone(),
                persistent: ty.persistent,
                synced: ty.synced,
                copy_on_death: ty.copy_on_death,
            }),
        )?;
        self.insert_dynamic(ty, value);
        if ty.synced {
            self.pending.insert(ty.key());
        }
        Ok(())
    }

    fn remove_dynamic(
        &mut self,
        ty: &DynamicAttachmentType,
    ) -> Result<Option<Compound>, AttachmentError> {
        let value = self.get_dynamic(ty)?;
        if value.is_some() {
            let key = ty.key();
            self.entries.remove(&key);
            if ty.synced {
                self.pending.insert(key);
            }
            self.refresh_budget(false)?;
        }
        Ok(value)
    }

    /// Whether a value is stored, saved or set, without decoding or inserting it.
    pub fn has<T: Clone + Send + Sync + 'static>(&self, ty: &AttachmentType<T>) -> bool {
        self.entries
            .contains_key(&(ty.namespace.into(), ty.name.into()))
    }
    /// Get a stored value without inserting the default; saved values decode lazily.
    pub fn get_existing<T: Clone + Send + Sync + 'static>(
        &mut self,
        ty: &AttachmentType<T>,
    ) -> Result<Option<T>, AttachmentError> {
        ty.validate_policy()?;
        if self
            .raw
            .get(&ty.namespace.into())
            .is_some_and(|tag| !matches!(tag, Tag::Compound(_)))
        {
            return Err(AttachmentError::InvalidNamespace);
        }
        let value = match self.entries.get(&(ty.namespace.into(), ty.name.into())) {
            Some(Entry::Raw(raw)) => {
                let value = raw_value(raw).ok_or(AttachmentError::Decode)?;
                (ty.codec.as_ref().ok_or(AttachmentError::Decode)?.decode)(value)?
            }
            Some(Entry::Decoded(value)) => {
                return value
                    .as_any()
                    .downcast_ref::<T>()
                    .cloned()
                    .map(Some)
                    .ok_or(AttachmentError::TypeMismatch);
            }
            Some(Entry::Dynamic(_)) => return Err(AttachmentError::TypeMismatch),
            None => return Ok(None),
        };
        self.insert_decoded(ty, value.clone())?;
        if let Some(Tag::Compound(names)) = self.raw.get_mut(&ty.namespace.into()) {
            names.remove(&ty.name.into());
        }
        Ok(Some(value))
    }
    /// Get an existing value or insert its default; saved values decode lazily.
    pub fn get<T: Clone + Send + Sync + 'static>(
        &mut self,
        ty: &AttachmentType<T>,
    ) -> Result<T, AttachmentError> {
        if let Some(value) = self.get_existing(ty)? {
            return Ok(value);
        }
        let value = (ty.default)();
        self.insert_decoded(ty, value.clone())?;
        if ty.sync {
            self.pending.insert((ty.namespace.into(), ty.name.into()));
        }
        Ok(value)
    }
    /// Replace a value. A key already decoded as a different type is rejected.
    pub fn set<T: Clone + Send + Sync + 'static>(
        &mut self,
        ty: &AttachmentType<T>,
        value: T,
    ) -> Result<(), AttachmentError> {
        ty.validate_policy()?;
        if self
            .raw
            .get(&ty.namespace.into())
            .is_some_and(|tag| !matches!(tag, Tag::Compound(_)))
        {
            return Err(AttachmentError::InvalidNamespace);
        }
        if let Some(Entry::Decoded(old)) = self.entries.get(&(ty.namespace.into(), ty.name.into()))
            && !old.as_any().is::<T>()
        {
            return Err(AttachmentError::TypeMismatch);
        }
        if matches!(
            self.entries.get(&(ty.namespace.into(), ty.name.into())),
            Some(Entry::Dynamic(_))
        ) {
            return Err(AttachmentError::TypeMismatch);
        }
        self.insert_decoded(ty, value)?;
        if ty.sync {
            self.pending.insert((ty.namespace.into(), ty.name.into()));
        }
        if let Some(Tag::Compound(names)) = self.raw.get_mut(&ty.namespace.into()) {
            names.remove(&ty.name.into());
        }
        Ok(())
    }
    /// Remove a value, retaining all other namespaces and names. A saved value
    /// that does not decode is removed too, and its error returned.
    pub fn remove<T: Clone + Send + Sync + 'static>(
        &mut self,
        ty: &AttachmentType<T>,
    ) -> Result<Option<T>, AttachmentError> {
        if matches!(
            self.entries.get(&(ty.namespace.into(), ty.name.into())),
            Some(Entry::Dynamic(_))
        ) {
            return Err(AttachmentError::TypeMismatch);
        }
        if !self.has(ty) {
            return Ok(None);
        }
        if ty.sync {
            self.pending.insert((ty.namespace.into(), ty.name.into()));
        }
        let value = self.get_existing(ty);
        self.entries.remove(&(ty.namespace.into(), ty.name.into()));
        if let Some(Tag::Compound(names)) = self.raw.get_mut(&ty.namespace.into()) {
            names.remove(&ty.name.into());
        }
        self.refresh_budget(false)?;
        value
    }
    fn insert_decoded<T: Clone + Send + Sync + 'static>(
        &mut self,
        ty: &AttachmentType<T>,
        value: T,
    ) -> Result<(), AttachmentError> {
        let encoded = ty
            .codec
            .as_ref()
            .map(|codec| (codec.encode)(&value))
            .transpose()?;
        if ty.sync {
            if let Some(value) = &encoded {
                validate_synced_value(ty.namespace, ty.name, value)?;
            }
        }
        let key = (ty.namespace.into(), ty.name.into());
        let entry = Entry::Decoded(Box::new(TypedValue {
            value,
            encoded,
            copy_on_death: ty.copy_on_death,
            sync: ty.sync,
        }));
        self.reserve(&key, &entry)?;
        self.entries.insert(key, entry);
        Ok(())
    }
    /// Queue a value to be sent to clients again; nothing for an unsynced type.
    pub fn mark_sync<T: 'static>(&mut self, ty: &AttachmentType<T>) -> Result<(), AttachmentError> {
        ty.validate_policy()?;
        if ty.sync {
            self.pending.insert((ty.namespace.into(), ty.name.into()));
        }
        Ok(())
    }
    /// Read queued changes without acknowledging their delivery.
    pub fn sync_changes(&self) -> Vec<AttachmentSync> {
        self.pending
            .iter()
            .cloned()
            .map(|(namespace, name)| {
                let value = self
                    .entries
                    .get(&(namespace.clone(), name.clone()))
                    .filter(|entry| entry.is_synced())
                    .and_then(Entry::synced_value)
                    .cloned();
                AttachmentSync {
                    namespace,
                    name,
                    value,
                }
            })
            .collect()
    }
    /// Acknowledge changes after they have been queued for every recipient.
    pub fn acknowledge_sync_changes(&mut self, changes: &[AttachmentSync]) {
        for change in changes {
            self.pending
                .remove(&(change.namespace.clone(), change.name.clone()));
        }
    }
    /// Take and acknowledge the current queued changes.
    pub fn take_sync_changes(&mut self) -> Vec<AttachmentSync> {
        let changes = self.sync_changes();
        self.acknowledge_sync_changes(&changes);
        changes
    }
    /// Every synced value, for a client that starts seeing the holder.
    pub fn synced(&self) -> Vec<AttachmentSync> {
        self.entries
            .iter()
            .filter(|(_, entry)| entry.is_synced())
            .filter_map(|((namespace, name), entry)| {
                Some(AttachmentSync {
                    namespace: namespace.clone(),
                    name: name.clone(),
                    value: Some(entry.synced_value()?.clone()),
                })
            })
            .collect()
    }
    /// Client side: keep what the server sent, decoded by the next `get`.
    pub fn apply_sync(&mut self, change: AttachmentSync) {
        let key = (change.namespace, change.name);
        if let Some(Tag::Compound(names)) = self.raw.get_mut(&key.0) {
            names.remove(&key.1);
        }
        match change.value {
            Some(value) => {
                let raw = [("value".into(), Tag::Compound(value))].into();
                self.entries.insert(key, Entry::Raw(raw));
            }
            None => {
                self.entries.remove(&key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DynamicHost(Attachments);

    impl AttachmentHost for DynamicHost {
        fn attachments(&mut self, _: Holder) -> Result<&mut Attachments, AttachmentError> {
            Ok(&mut self.0)
        }
    }

    fn dynamic_policy(scope: AttachmentScope) -> DynamicAttachmentPolicy {
        DynamicAttachmentPolicy {
            scope,
            persistent: true,
            synced: false,
            copy_on_death: false,
        }
    }

    fn budget_host(registry: &DynamicAttachmentRegistry, holder: &str) -> DynamicHost {
        let mut host = DynamicHost(Attachments::default());
        registry.bind(&mut host.0, holder.to_owned());
        Arc::get_mut(host.0.budget.as_mut().unwrap()).unwrap().limit = 1024;
        host
    }

    fn payload(bytes: usize) -> Compound {
        [("value".into(), Tag::ByteArray(vec![0; bytes]))].into()
    }

    #[test]
    fn budget_is_shared_across_holders_but_not_packages_or_worlds() {
        let mut registry = DynamicAttachmentRegistry::default();
        let policy = dynamic_policy(AttachmentScope::Player);
        let first = registry.define("test", "test:first", policy).unwrap();
        let other = registry.define("other", "other:first", policy).unwrap();
        let mut first_host = budget_host(&registry, "player:1");
        let mut second_host = budget_host(&registry, "player:2");
        first
            .set(&mut first_host, Holder::Player(1), payload(600))
            .unwrap();
        assert_eq!(
            first.set(&mut second_host, Holder::Player(2), payload(600)),
            Err(AttachmentError::QuotaExceeded)
        );
        assert_eq!(first.get(&mut second_host, Holder::Player(2)), Ok(None));
        other
            .set(&mut second_host, Holder::Player(2), payload(600))
            .unwrap();
        let fresh_registry = DynamicAttachmentRegistry::default();
        let mut fresh_host = budget_host(&fresh_registry, "player:1");
        first
            .set(&mut fresh_host, Holder::Player(1), payload(600))
            .unwrap();
    }

    #[test]
    fn budget_replacements_and_removals_are_atomic() {
        let mut registry = DynamicAttachmentRegistry::default();
        let definition = registry
            .define("test", "test:value", dynamic_policy(AttachmentScope::Level))
            .unwrap();
        let mut host = budget_host(&registry, "level:test");
        definition
            .set(&mut host, Holder::Level, payload(600))
            .unwrap();
        definition
            .set(&mut host, Holder::Level, payload(700))
            .unwrap();
        assert_eq!(
            definition.set(&mut host, Holder::Level, payload(1000)),
            Err(AttachmentError::QuotaExceeded)
        );
        assert_eq!(
            definition.get(&mut host, Holder::Level),
            Ok(Some(payload(700)))
        );
        definition.remove(&mut host, Holder::Level).unwrap();
        definition
            .set(&mut host, Holder::Level, payload(900))
            .unwrap();
    }

    #[test]
    fn budget_keeps_unloaded_persistent_data_and_releases_memory_only_data() {
        let mut registry = DynamicAttachmentRegistry::default();
        let policy = dynamic_policy(AttachmentScope::Player);
        let persistent = registry.define("test", "test:saved", policy).unwrap();
        let memory = registry
            .define(
                "test",
                "test:memory",
                DynamicAttachmentPolicy {
                    persistent: false,
                    ..policy
                },
            )
            .unwrap();
        let mut saved_host = budget_host(&registry, "player:1");
        persistent
            .set(&mut saved_host, Holder::Player(1), payload(600))
            .unwrap();
        let saved = saved_host.0.to_nbt();
        drop(saved_host);
        let mut host = budget_host(&registry, "player:2");
        assert_eq!(
            memory.set(&mut host, Holder::Player(2), payload(600)),
            Err(AttachmentError::QuotaExceeded)
        );
        let mut reloaded = DynamicHost(Attachments::from_nbt(saved));
        registry.bind(&mut reloaded.0, "player:1".into());
        persistent.remove(&mut reloaded, Holder::Player(1)).unwrap();
        memory
            .set(&mut host, Holder::Player(2), payload(600))
            .unwrap();
        drop(host);
        let mut next = budget_host(&registry, "player:3");
        memory
            .set(&mut next, Holder::Player(3), payload(600))
            .unwrap();
    }

    #[test]
    fn budget_counts_cold_unknown_and_malformed_saved_data() {
        let mut registry = DynamicAttachmentRegistry::default();
        let definition = registry
            .define("test", "test:new", dynamic_policy(AttachmentScope::Level))
            .unwrap();
        registry.account_saved(
            "level:cold".into(),
            [("test".into(), Tag::ByteArray(vec![0; 900]))].into(),
        );
        let mut host = budget_host(&registry, "level:live");
        assert_eq!(
            definition.set(&mut host, Holder::Level, payload(100)),
            Err(AttachmentError::QuotaExceeded)
        );
        registry.account_saved("level:cold".into(), Compound::new());
        let unknown = Attachments::from_nbt(
            [(
                "test".into(),
                Tag::compound([(
                    "old",
                    Tag::compound([("data", Tag::Compound(payload(900)))]),
                )]),
            )]
            .into(),
        );
        registry.account_saved("level:cold".into(), unknown.to_nbt());
        assert_eq!(
            definition.set(&mut host, Holder::Level, payload(100)),
            Err(AttachmentError::QuotaExceeded)
        );
    }

    #[test]
    fn budget_snapshots_do_not_own_the_live_holders_reservation() {
        let mut registry = DynamicAttachmentRegistry::default();
        let policy = DynamicAttachmentPolicy {
            persistent: false,
            ..dynamic_policy(AttachmentScope::Level)
        };
        let definition = registry.define("test", "test:value", policy).unwrap();
        let mut live = budget_host(&registry, "level:one");
        definition
            .set(&mut live, Holder::Level, payload(600))
            .unwrap();
        let snapshot = live.0.clone();
        assert!(snapshot.budget.is_none());
        let mut replacement = DynamicHost(live.0.clone());
        registry.bind(&mut replacement.0, "level:one".into());
        drop(live);
        let mut other = budget_host(&registry, "level:two");
        assert_eq!(
            definition.set(&mut other, Holder::Level, payload(600)),
            Err(AttachmentError::QuotaExceeded)
        );
        drop(replacement);
        definition
            .set(&mut other, Holder::Level, payload(600))
            .unwrap();
    }

    #[test]
    fn incomplete_saved_usage_index_refuses_growth_but_allows_shrinking() {
        let mut registry = DynamicAttachmentRegistry::default();
        let definition = registry
            .define("test", "test:value", dynamic_policy(AttachmentScope::Level))
            .unwrap();
        let mut host = budget_host(&registry, "level:one");
        definition
            .set(&mut host, Holder::Level, payload(600))
            .unwrap();
        registry.deny_growth();
        definition
            .set(&mut host, Holder::Level, payload(500))
            .unwrap();
        assert_eq!(
            definition.set(&mut host, Holder::Level, payload(600)),
            Err(AttachmentError::QuotaExceeded)
        );
        definition.remove(&mut host, Holder::Level).unwrap();
    }

    #[test]
    fn dynamic_definitions_check_owner_identifier_and_holder_scope() {
        let mut registry = DynamicAttachmentRegistry::default();
        let policy = dynamic_policy(AttachmentScope::Player);
        let definition = registry
            .define("test", "test:stats/health", policy)
            .unwrap();
        assert!(registry.get("test", "test:stats/health").is_some());
        assert!(registry.get("other", "test:stats/health").is_none());
        assert!(matches!(
            registry.define("test", "test:stats/health", policy),
            Err(AttachmentError::DuplicateType)
        ));
        assert!(matches!(
            registry.define("other", "test:value", policy),
            Err(AttachmentError::WrongOwner)
        ));
        for invalid in [
            "test",
            "test:",
            "test:../value",
            "test:has space",
            "Test:value",
        ] {
            assert!(matches!(
                registry.define("test", invalid, policy),
                Err(AttachmentError::InvalidId)
            ));
        }
        let mut host = DynamicHost(Attachments::default());
        assert_eq!(definition.get(&mut host, Holder::Player(1)), Ok(None));
        assert_eq!(
            definition.get(&mut host, Holder::Entity(1)),
            Err(AttachmentError::ScopeMismatch)
        );
    }

    #[test]
    fn dynamic_ids_are_limited_by_bytes() {
        let mut registry = DynamicAttachmentRegistry::default();
        let policy = dynamic_policy(AttachmentScope::Level);
        let longest = format!("test:{}", "a".repeat(123));
        assert_eq!(longest.len(), MAX_ATTACHMENT_ID_BYTES);
        assert!(registry.define("test", &longest, policy).is_ok());
        assert!(matches!(
            registry.define("test", &(longest + "a"), policy),
            Err(AttachmentError::InvalidId)
        ));
    }

    #[test]
    fn saving_skips_unencodable_entries_without_losing_valid_unknown_data() {
        let invalid_key = "x".repeat(65_536);
        let raw: Compound = [(
            "missing".into(),
            Tag::Compound(
                [
                    ("valid".into(), Tag::compound([("data", Tag::Int(42))])),
                    (
                        invalid_key.as_str().into(),
                        Tag::compound([("data", Tag::Int(1))]),
                    ),
                    ("invalid_value".into(), Tag::compound([("data", Tag::End)])),
                ]
                .into(),
            ),
        )]
        .into();
        let saved = Attachments::from_nbt(raw).to_nbt();
        let Tag::Compound(entries) = saved.get(&"missing".into()).unwrap() else {
            panic!("namespace retained");
        };
        assert_eq!(entries.len(), 1);
        assert!(entries.contains_key(&"valid".into()));
        let root = crate::nbt::NamedTag {
            name: "".into(),
            tag: Tag::Compound(saved),
        };
        assert!(crate::nbt::write_named(&root, crate::nbt::Limits::saved()).is_ok());
    }

    #[test]
    fn dynamic_flags_independently_control_persistence_copy_and_sync() {
        for persistent in [false, true] {
            for synced in [false, true] {
                for copy_on_death in [false, true] {
                    let mut registry = DynamicAttachmentRegistry::default();
                    let definition = registry
                        .define(
                            "test",
                            "test:data",
                            DynamicAttachmentPolicy {
                                scope: AttachmentScope::Player,
                                persistent,
                                synced,
                                copy_on_death,
                            },
                        )
                        .unwrap();
                    let mut host = DynamicHost(Attachments::default());
                    let value = encode(&42).unwrap();
                    definition
                        .set(&mut host, Holder::Player(1), value.clone())
                        .unwrap();
                    assert_eq!(
                        definition.get(&mut host, Holder::Player(1)),
                        Ok(Some(value.clone()))
                    );
                    let change_count = if synced { 1 } else { 0 };
                    assert_eq!(host.0.synced().len(), change_count);
                    assert_eq!(host.0.take_sync_changes().len(), change_count);
                    let mut loaded = DynamicHost(Attachments::from_nbt(host.0.to_nbt()));
                    assert_eq!(
                        definition.get(&mut loaded, Holder::Player(1)),
                        Ok(persistent.then(|| value.clone()))
                    );
                    let mut copied_loaded = DynamicHost(loaded.0.copy_on_death().unwrap());
                    assert_eq!(
                        definition.get(&mut copied_loaded, Holder::Player(2)),
                        Ok((persistent && copy_on_death).then(|| value.clone()))
                    );
                    let mut copied = DynamicHost(host.0.copy_on_death().unwrap());
                    assert_eq!(
                        definition.get(&mut copied, Holder::Player(2)),
                        Ok(copy_on_death.then(|| value.clone()))
                    );
                    assert_eq!(
                        definition.remove(&mut host, Holder::Player(1)),
                        Ok(Some(value))
                    );
                    let changes = host.0.take_sync_changes();
                    assert_eq!(changes.len(), change_count);
                    if synced {
                        assert!(changes[0].value.is_none());
                    }
                    assert_eq!(definition.get(&mut host, Holder::Player(1)), Ok(None));
                }
            }
        }
    }

    #[test]
    fn dynamic_reconcile_applies_current_policy_to_saved_values() {
        let holder = Holder::Player(1);
        let mut original = DynamicAttachmentRegistry::default();
        let definition = original
            .define(
                "test",
                "test:profile",
                DynamicAttachmentPolicy {
                    scope: AttachmentScope::Player,
                    persistent: true,
                    synced: true,
                    copy_on_death: true,
                },
            )
            .unwrap();
        let mut host = DynamicHost(Attachments::default());
        definition.set(&mut host, holder, encode(&42).unwrap()).unwrap();
        let mut loaded = DynamicHost(Attachments::from_nbt(host.0.to_nbt()));

        let mut current = DynamicAttachmentRegistry::default();
        let current_definition = current
            .define(
                "test",
                "test:profile",
                DynamicAttachmentPolicy {
                    scope: AttachmentScope::Player,
                    persistent: true,
                    synced: false,
                    copy_on_death: false,
                },
            )
            .unwrap();
        current.reconcile(&mut loaded, holder).unwrap();
        assert!(loaded.0.synced().is_empty());
        let mut copied = DynamicHost(loaded.0.copy_on_death().unwrap());
        assert_eq!(current_definition.get(&mut copied, holder), Ok(None));
        assert_eq!(
            current_definition.get(&mut loaded, holder),
            Ok(Some(encode(&42).unwrap()))
        );

        let mut transient = DynamicAttachmentRegistry::default();
        let transient_definition = transient
            .define(
                "test",
                "test:profile",
                DynamicAttachmentPolicy {
                    scope: AttachmentScope::Player,
                    persistent: false,
                    synced: false,
                    copy_on_death: false,
                },
            )
            .unwrap();
        transient.reconcile(&mut loaded, holder).unwrap();
        let mut reloaded = DynamicHost(Attachments::from_nbt(loaded.0.to_nbt()));
        assert_eq!(transient_definition.get(&mut reloaded, holder), Ok(None));
    }

    #[test]
    fn dynamic_sync_delivers_snapshots_updates_and_removals() {
        let mut registry = DynamicAttachmentRegistry::default();
        let definition = registry
            .define(
                "test",
                "test:entity_data",
                DynamicAttachmentPolicy {
                    scope: AttachmentScope::Entity,
                    persistent: true,
                    synced: true,
                    copy_on_death: false,
                },
            )
            .unwrap();
        let holder = Holder::Entity(7);
        let mut server = DynamicHost(Attachments::default());
        let mut client = DynamicHost(Attachments::default());
        definition.set(&mut server, holder, encode(&1).unwrap()).unwrap();
        let mut reloaded_client = DynamicHost(Attachments::from_nbt(server.0.to_nbt()));
        for change in server.0.synced() {
            client.0.apply_sync(change);
        }
        assert_eq!(definition.get(&mut client, holder), Ok(Some(encode(&1).unwrap())));
        server.0.take_sync_changes();
        definition.set(&mut server, holder, encode(&2).unwrap()).unwrap();
        for change in server.0.take_sync_changes() {
            client.0.apply_sync(change);
        }
        assert_eq!(definition.get(&mut client, holder), Ok(Some(encode(&2).unwrap())));
        definition.remove(&mut server, holder).unwrap();
        for change in server.0.take_sync_changes() {
            reloaded_client.0.apply_sync(change.clone());
            client.0.apply_sync(change);
        }
        assert_eq!(definition.get(&mut client, holder), Ok(None));
        let mut saved_client = DynamicHost(Attachments::from_nbt(reloaded_client.0.to_nbt()));
        assert_eq!(definition.get(&mut saved_client, holder), Ok(None));
    }

    #[test]
    fn dynamic_values_reject_typed_collisions_and_preserve_malformed_data() {
        let mut registry = DynamicAttachmentRegistry::default();
        let definition = registry
            .define("test", "test:counter", dynamic_policy(AttachmentScope::Level))
            .unwrap();
        let mut host = DynamicHost(Attachments::default());
        host.0.set(&COUNTER, 1).unwrap();
        assert_eq!(
            definition.get(&mut host, Holder::Level),
            Err(AttachmentError::TypeMismatch)
        );
        assert_eq!(
            definition.set(&mut host, Holder::Level, encode(&2).unwrap()),
            Err(AttachmentError::TypeMismatch)
        );
        assert_eq!(host.0.get(&COUNTER), Ok(1));
        let mut host = DynamicHost(Attachments::default());
        definition
            .set(&mut host, Holder::Level, encode(&2).unwrap())
            .unwrap();
        assert_eq!(host.0.get(&COUNTER), Err(AttachmentError::TypeMismatch));
        assert_eq!(host.0.remove(&COUNTER), Err(AttachmentError::TypeMismatch));
        let raw: Compound = [(
            "test".into(),
            Tag::Compound([("counter".into(), Tag::Int(5))].into()),
        )]
        .into();
        let mut host = DynamicHost(Attachments::from_nbt(raw.clone()));
        assert_eq!(
            definition.set(&mut host, Holder::Level, encode(&3).unwrap()),
            Err(AttachmentError::Decode)
        );
        assert_eq!(host.0.to_nbt(), raw);
    }

    fn encode(n: &i32) -> Result<Compound, AttachmentError> {
        Ok([(JavaString::from("value"), Tag::Int(*n))].into())
    }
    fn decode(c: &Compound) -> Result<i32, AttachmentError> {
        match c.get(&"value".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        }
    }
    static COUNTER: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "counter",
        default: || 0,
        codec: Some(AttachmentCodec { encode, decode }),
        copy_on_death: false,
        sync: false,
    };
    static SYNCED: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "synced",
        default: || 7,
        codec: Some(AttachmentCodec { encode, decode }),
        copy_on_death: false,
        sync: true,
    };
    fn change(name: &str, value: Option<i32>) -> AttachmentSync {
        AttachmentSync {
            namespace: "test".into(),
            name: name.into(),
            value: value.map(|n| encode(&n).unwrap()),
        }
    }
    static MEMORY: AttachmentType<i32> = AttachmentType {
        namespace: "test",
        name: "memory",
        default: || 3,
        codec: None,
        copy_on_death: false,
        sync: false,
    };
    #[test]
    fn saved_values_round_trip_and_memory_values_do_not() {
        let mut map = Attachments::default();
        map.set(&COUNTER, 42).unwrap();
        map.set(&MEMORY, 8).unwrap();
        let mut loaded = Attachments::from_nbt(map.to_nbt());
        assert_eq!(loaded.get(&COUNTER), Ok(42));
        assert_eq!(loaded.get(&MEMORY), Ok(3));
        assert_eq!(loaded.remove(&COUNTER), Ok(Some(42)));
        let mut reopened = Attachments::from_nbt(loaded.to_nbt());
        assert_eq!(reopened.get(&COUNTER), Ok(0));
    }
    #[test]
    fn unread_saved_values_are_copied_raw_and_never_typed_by_their_flags() {
        let keep = AttachmentType {
            namespace: "test",
            name: "keep",
            default: || 0,
            codec: Some(AttachmentCodec { encode, decode }),
            copy_on_death: true,
            sync: false,
        };
        let mut original = Attachments::default();
        original.set(&keep, 42).unwrap();
        original.set(&COUNTER, 5).unwrap();
        let mut loaded = Attachments::from_nbt(original.to_nbt());
        // Still unread: `keep` is registered (it was set above) and opts in, so
        // it is carried over uninterpreted; COUNTER is registered as not copied.
        let mut respawned = loaded.copy_on_death().unwrap();
        assert_eq!(respawned.get(&keep), Ok(42));
        assert_eq!(respawned.get(&COUNTER), Ok(0));
        let mut loaded = Attachments::from_nbt(original.to_nbt());
        // A read value follows its current type: COUNTER opts out of copying.
        assert_eq!(loaded.get_existing(&COUNTER), Ok(Some(5)));
        let mut respawned = loaded.copy_on_death().unwrap();
        assert_eq!(respawned.get(&keep), Ok(42));
        assert_eq!(respawned.get(&COUNTER), Ok(0));
        // The raw value is not typed from saved flags: a different codec sees it as is.
        assert_eq!(
            Attachments::from_nbt(original.to_nbt())
                .copy_on_death()
                .unwrap()
                .synced(),
            []
        );
    }

    #[test]
    fn unread_dynamic_values_follow_their_registered_policy_on_respawn() {
        let holder = Holder::Player(1);
        let mut registry = DynamicAttachmentRegistry::default();
        let define = |registry: &mut DynamicAttachmentRegistry, id: &str, copy_on_death| {
            registry
                .define(
                    id.split(':').next().unwrap(),
                    id,
                    DynamicAttachmentPolicy {
                        scope: AttachmentScope::Player,
                        persistent: true,
                        synced: false,
                        copy_on_death,
                    },
                )
                .unwrap()
        };
        let kept = define(&mut registry, "test:kept", true);
        let dropped = define(&mut registry, "test:dropped", false);
        let mut host = DynamicHost(Attachments::default());
        kept.set(&mut host, holder, payload(4)).unwrap();
        dropped.set(&mut host, holder, payload(5)).unwrap();
        let mut raw = host.0.to_nbt();
        // A value whose type is not defined (its mod is not loaded).
        raw.insert(
            "gone".into(),
            Tag::Compound(
                [(
                    "x".into(),
                    Tag::Compound([("value".into(), Tag::Compound(payload(3)))].into()),
                )]
                .into(),
            ),
        );
        // Nothing has read any of them since loading.
        let mut loaded = Attachments::from_nbt(raw);
        let mut copied = DynamicHost(registry.copy_on_death(holder, &mut loaded).unwrap());
        assert_eq!(kept.get(&mut copied, holder), Ok(Some(payload(4))));
        assert_eq!(dropped.get(&mut copied, holder), Ok(None));
        // The unknown value stayed raw (and is saved as it was), then reads
        // once its type is defined.
        let mut saved = DynamicHost(Attachments::from_nbt(copied.0.to_nbt()));
        let late = define(&mut registry, "gone:x", false);
        assert_eq!(late.get(&mut saved, holder), Ok(Some(payload(3))));
    }

    #[test]
    fn unread_static_values_follow_the_static_registry_on_respawn() {
        let make = |name: &'static str, copy_on_death| AttachmentType {
            namespace: "statreg",
            name,
            default: || 0,
            codec: Some(AttachmentCodec { encode, decode }),
            copy_on_death,
            sync: false,
        };
        let (keep, drop_it, unknown) = (make("keep", true), make("drop", false), make("late", true));
        let mut original = Attachments::default();
        original.set(&keep, 1).unwrap();
        original.set(&drop_it, 2).unwrap();
        // `unknown` is never used, only saved by a raw entry, so it is not registered.
        let mut raw = original.to_nbt();
        let Some(Tag::Compound(names)) = raw.get_mut(&"statreg".into()) else {
            panic!("namespace saved");
        };
        names.insert(
            "other".into(),
            Tag::Compound([("value".into(), Tag::Compound(encode(&7).unwrap()))].into()),
        );
        // Nothing reads any value after loading.
        let loaded = Attachments::from_nbt(raw);
        let mut copy = loaded.copy_on_death().unwrap();
        assert!(copy.has(&keep));
        assert!(!copy.has(&drop_it));
        assert_eq!(copy.get_existing(&keep), Ok(Some(1)));
        let other = AttachmentType { name: "other", ..make("other", false) };
        assert!(copy.has(&other), "an unregistered type is carried raw");
        let _ = unknown;
    }

    #[test]
    fn conflicting_static_registrations_panic() {
        let make = |name: &'static str, copy_on_death| AttachmentType::<i32> {
            namespace: "statclash",
            name,
            default: || 0,
            codec: Some(AttachmentCodec { encode, decode }),
            copy_on_death,
            sync: false,
        };
        register_static(&make("same", true));
        register_static(&make("same", true));
        assert!(std::panic::catch_unwind(|| register_static(&make("same", false))).is_err());
        let mut registry = DynamicAttachmentRegistry::default();
        let overlapping = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = registry.define(
                "statclash",
                "statclash:same",
                DynamicAttachmentPolicy {
                    scope: AttachmentScope::Player,
                    persistent: true,
                    synced: false,
                    copy_on_death: false,
                },
            );
        }));
        assert!(overlapping.is_err());
    }

    #[test]
    fn existing_reads_leave_an_absent_value_absent() {
        let mut map = Attachments::default();
        assert!(!map.has(&COUNTER));
        assert_eq!(map.get_existing(&COUNTER), Ok(None));
        assert!(!map.has(&COUNTER));
        assert_eq!(map.to_nbt(), Compound::new());
        assert_eq!(map.get(&COUNTER), Ok(0));
        assert!(map.has(&COUNTER));
        assert_ne!(map.to_nbt(), Compound::new());
        map.set(&COUNTER, 5).unwrap();
        assert_eq!(map.get_existing(&COUNTER), Ok(Some(5)));
    }
    #[test]
    fn existing_read_decodes_a_saved_value_without_changing_it() {
        let mut map = Attachments::default();
        map.set(&COUNTER, 42).unwrap();
        map.set(&MEMORY, 8).unwrap();
        let saved = map.to_nbt();
        let mut loaded = Attachments::from_nbt(saved.clone());
        assert!(loaded.has(&COUNTER));
        assert_eq!(loaded.get_existing(&COUNTER), Ok(Some(42)));
        assert_eq!(loaded.to_nbt(), saved);
        assert!(!loaded.has(&MEMORY));
        assert_eq!(loaded.get_existing(&MEMORY), Ok(None));
    }
    #[test]
    fn unknown_and_invalid_values_survive_a_failed_decode() {
        let saved: Compound = [(
            JavaString::from("test"),
            Tag::Compound(
                [
                    (
                        JavaString::from("counter"),
                        Tag::Compound(
                            [(JavaString::from("value"), Tag::String("bad".into()))].into(),
                        ),
                    ),
                    (JavaString(vec![0xd800]), Tag::Compound(Compound::new())),
                ]
                .into(),
            ),
        )]
        .into();
        let mut map = Attachments::from_nbt(saved.clone());
        assert!(map.has(&COUNTER));
        assert_eq!(map.get_existing(&COUNTER), Err(AttachmentError::Decode));
        assert_eq!(map.get(&COUNTER), Err(AttachmentError::Decode));
        assert_eq!(map.to_nbt(), saved);
        assert_eq!(map.remove(&COUNTER), Err(AttachmentError::Decode));
        assert_eq!(map.remove(&COUNTER), Ok(None), "removed all the same");
    }
    #[test]
    fn synced_set_and_remove_report_the_last_state_once() {
        let mut map = Attachments::default();
        map.set(&SYNCED, 1).unwrap();
        map.set(&SYNCED, 2).unwrap();
        map.remove(&SYNCED).unwrap();
        assert_eq!(map.take_sync_changes(), [change("synced", None)]);
        assert!(map.take_sync_changes().is_empty());
    }
    #[test]
    fn pending_sync_changes_remain_until_acknowledged() {
        let mut map = Attachments::default();
        map.set(&SYNCED, 2).unwrap();
        let changes = map.sync_changes();
        assert_eq!(changes, [change("synced", Some(2))]);
        assert_eq!(map.sync_changes(), changes);
        map.acknowledge_sync_changes(&changes);
        assert!(map.sync_changes().is_empty());
    }
    #[test]
    fn oversized_synced_dynamic_values_are_rejected_without_a_change() {
        let mut registry = DynamicAttachmentRegistry::default();
        let definition = registry
            .define(
                "test",
                "test:large",
                DynamicAttachmentPolicy {
                    synced: true,
                    ..dynamic_policy(AttachmentScope::Player)
                },
            )
            .unwrap();
        let mut host = DynamicHost(Attachments::default());
        let value = [(
            JavaString::from("data"),
            Tag::ByteArray(vec![0; MAX_SYNCED_ATTACHMENT_BYTES + 1]),
        )]
        .into();
        assert!(definition.set(&mut host, Holder::Player(1), value).is_err());
        assert!(host.0.sync_changes().is_empty());
        assert_eq!(definition.get(&mut host, Holder::Player(1)), Ok(None));
    }
    #[test]
    fn default_creation_is_synced_and_a_later_read_is_not() {
        let mut map = Attachments::default();
        assert_eq!(map.get(&SYNCED), Ok(7));
        assert_eq!(map.take_sync_changes(), [change("synced", Some(7))]);
        assert_eq!(map.get(&SYNCED), Ok(7));
        assert!(map.take_sync_changes().is_empty());
    }
    #[test]
    fn unsynced_types_report_no_changes() {
        let mut map = Attachments::default();
        map.set(&COUNTER, 1).unwrap();
        map.remove(&COUNTER).unwrap();
        map.mark_sync(&COUNTER).unwrap();
        assert!(map.take_sync_changes().is_empty());
    }
    #[test]
    fn sync_without_a_codec_is_refused() {
        let broken = AttachmentType {
            namespace: "test",
            name: "broken",
            default: || 0,
            codec: None,
            copy_on_death: false,
            sync: true,
        };
        let mut map = Attachments::default();
        assert_eq!(map.get(&broken), Err(AttachmentError::InvalidSyncPolicy));
        assert_eq!(map.set(&broken, 1), Err(AttachmentError::InvalidSyncPolicy));
        assert_eq!(
            map.mark_sync(&broken),
            Err(AttachmentError::InvalidSyncPolicy)
        );
        assert!(map.take_sync_changes().is_empty());
    }
    #[test]
    fn saved_flags_do_not_sync_unknown_types() {
        let mut map = Attachments::default();
        map.set(&SYNCED, 5).unwrap();
        map.set(&COUNTER, 6).unwrap();
        let saved = map.to_nbt();
        let Some(Tag::Compound(names)) = saved.get(&"test".into()) else {
            panic!("namespace saved");
        };
        let Some(Tag::Compound(envelope)) = names.get(&"synced".into()) else {
            panic!("entry saved");
        };
        assert_eq!(envelope.get(&"sync".into()), Some(&Tag::Byte(1)));
        let mut loaded = Attachments::from_nbt(saved);
        assert!(loaded.synced().is_empty());
        assert_eq!(loaded.get_existing(&SYNCED), Ok(Some(5)));
        assert_eq!(loaded.synced(), [change("synced", Some(5))]);
    }
    #[test]
    fn applied_changes_decode_through_the_codec_and_removals_clear() {
        let mut map = Attachments::default();
        map.apply_sync(change("synced", Some(9)));
        assert_eq!(map.get(&SYNCED), Ok(9));
        map.apply_sync(change("synced", None));
        assert_eq!(map.get(&SYNCED), Ok(7));
    }
    #[test]
    fn sync_data_reports_the_current_value() {
        struct One(Attachments);
        impl AttachmentHost for One {
            fn attachments(&mut self, _: Holder) -> Result<&mut Attachments, AttachmentError> {
                Ok(&mut self.0)
            }
        }
        let mut host = One(Attachments::default());
        SYNCED.set(&mut host, Holder::Level, 4).unwrap();
        host.0.take_sync_changes();
        SYNCED.sync_data(&mut host, Holder::Level).unwrap();
        assert_eq!(host.0.take_sync_changes(), [change("synced", Some(4))]);
    }
}
