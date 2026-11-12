use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use horizon_client::mod_api::{BlockPos, Direction, Host, ItemHandler, ItemStack};

use super::Game;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CapabilityKind {
    Items,
    Fluids,
    Energy,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CapabilityTarget {
    Block(String),
    BlockEntity(String),
    Item(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FluidStack {
    pub fluid: String,
    pub amount: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FluidTank {
    pub capacity: u32,
    pub contents: Option<FluidStack>,
}

pub trait FluidHandler: Send + Sync {
    fn tanks(&self) -> usize;
    fn tank(&self, index: usize) -> Option<FluidTank>;
    fn fill(&self, stack: &FluidStack, simulate: bool) -> u32;
    fn drain(&self, fluid: &str, amount: u32, simulate: bool) -> Option<FluidStack>;
}

pub struct FluidStorage(Mutex<Vec<FluidTank>>);

impl FluidStorage {
    pub fn new(capacities: impl IntoIterator<Item = u32>) -> Self {
        Self(Mutex::new(
            capacities
                .into_iter()
                .map(|capacity| FluidTank {
                    capacity,
                    contents: None,
                })
                .collect(),
        ))
    }
}

impl FluidHandler for FluidStorage {
    fn tanks(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    fn tank(&self, index: usize) -> Option<FluidTank> {
        self.0.lock().unwrap().get(index).cloned()
    }

    fn fill(&self, stack: &FluidStack, simulate: bool) -> u32 {
        if stack.amount == 0 || stack.fluid.is_empty() {
            return 0;
        }
        let mut tanks = self.0.lock().unwrap();
        let mut remaining = stack.amount;
        for tank in tanks.iter_mut() {
            if tank
                .contents
                .as_ref()
                .is_some_and(|stored| stored.fluid != stack.fluid)
            {
                continue;
            }
            let stored = tank.contents.as_ref().map_or(0, |stored| stored.amount);
            let accepted = remaining.min(tank.capacity.saturating_sub(stored));
            if accepted > 0 && !simulate {
                tank.contents = Some(FluidStack {
                    fluid: stack.fluid.clone(),
                    amount: stored + accepted,
                });
            }
            remaining -= accepted;
            if remaining == 0 {
                break;
            }
        }
        stack.amount - remaining
    }

    fn drain(&self, fluid: &str, amount: u32, simulate: bool) -> Option<FluidStack> {
        let mut tanks = self.0.lock().unwrap();
        let mut remaining = amount;
        for tank in tanks.iter_mut() {
            let Some(stored) = tank.contents.as_mut().filter(|stored| stored.fluid == fluid) else {
                continue;
            };
            let drained = remaining.min(stored.amount);
            remaining -= drained;
            if !simulate {
                stored.amount -= drained;
                if stored.amount == 0 {
                    tank.contents = None;
                }
            }
            if remaining == 0 {
                break;
            }
        }
        (remaining < amount).then(|| FluidStack {
            fluid: fluid.to_owned(),
            amount: amount - remaining,
        })
    }
}

#[derive(Clone)]
pub enum Capability {
    Items(Arc<dyn ItemHandler>),
    Fluids(Arc<dyn FluidHandler>),
    Energy,
}

impl Capability {
    fn kind(&self) -> CapabilityKind {
        match self {
            Self::Items(_) => CapabilityKind::Items,
            Self::Fluids(_) => CapabilityKind::Fluids,
            Self::Energy => CapabilityKind::Energy,
        }
    }
}

pub struct CapabilityContext {
    pub position: Option<BlockPos>,
    pub side: Option<Direction>,
    pub stack: Option<ItemStack>,
}

pub trait CapabilityProvider: Send + Sync {
    fn provide(&self, host: &mut dyn Host, context: &CapabilityContext) -> Option<Capability>;
}

pub struct NativeCapabilityProvider {
    pub target: CapabilityTarget,
    pub kind: CapabilityKind,
    pub provider: Arc<dyn CapabilityProvider>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityEnergy {
    pub amount: i32,
    pub capacity: i32,
    pub max_insert: i32,
    pub max_extract: i32,
    pub can_insert: bool,
    pub can_extract: bool,
}

struct Providers(HashMap<(CapabilityTarget, CapabilityKind), Arc<dyn CapabilityProvider>>);

impl Providers {
    fn new(entries: Vec<NativeCapabilityProvider>) -> Result<Self, String> {
        let mut providers = HashMap::new();
        for entry in entries {
            let id = match &entry.target {
                CapabilityTarget::Block(id)
                | CapabilityTarget::BlockEntity(id)
                | CapabilityTarget::Item(id) => id,
            };
            let valid = if let Some(path) = id.strip_prefix("minecraft:") {
                super::native_block_behaviors::valid_id(&format!("capability:{path}"))
            } else {
                super::native_block_behaviors::valid_id(id)
            };
            if !valid {
                return Err(format!("invalid capability target: {id}"));
            }
            if providers
                .insert((entry.target, entry.kind), entry.provider)
                .is_some()
            {
                return Err("duplicate capability target and kind".to_owned());
            }
        }
        Ok(Self(providers))
    }

    fn get(
        &self,
        target: CapabilityTarget,
        kind: CapabilityKind,
    ) -> Option<Arc<dyn CapabilityProvider>> {
        self.0.get(&(target, kind)).cloned()
    }
}

static PROVIDERS: OnceLock<Providers> = OnceLock::new();

fn provider_for(
    registry: &OnceLock<Providers>,
    kind: CapabilityKind,
    targets: impl FnOnce() -> Option<(CapabilityTarget, Option<CapabilityTarget>)>,
) -> Option<Arc<dyn CapabilityProvider>> {
    let providers = registry.get()?;
    let (primary, fallback) = targets()?;
    providers
        .get(primary, kind)
        .or_else(|| fallback.and_then(|target| providers.get(target, kind)))
}

pub(super) fn register() -> Result<(), String> {
    let mut providers = Vec::new();
    Game::native_capability_providers(&mut providers);
    let providers = Providers::new(providers)?;
    if providers.0.is_empty() {
        return Ok(());
    }
    PROVIDERS
        .set(providers)
        .map_err(|_| "capability providers already registered".to_owned())
}

pub(super) fn insert_item(
    handler: &dyn ItemHandler,
    stack: ItemStack,
    simulate: bool,
) -> Option<ItemStack> {
    let mut remaining = stack;
    for slot in 0..handler.slots() {
        remaining = handler.insert(slot, remaining, simulate)?;
    }
    Some(remaining)
}

#[allow(dead_code)]
impl Game<'_> {
    pub(super) fn native_query_capability(
        &mut self,
        position: BlockPos,
        side: Option<Direction>,
        kind: CapabilityKind,
    ) -> Option<Capability> {
        let provider = provider_for(&PROVIDERS, kind, || {
            if !self.native_simulates() {
                return None;
            }
            let state = self.world.block(position.x, position.y, position.z);
            let block = CapabilityTarget::Block(self.pack.assets.registry.state(state)?.name.to_string());
            let entity = self
                .world
                .block_entities
                .get(&(position.x, position.y, position.z))
                .map(|entity| CapabilityTarget::BlockEntity(entity.id().to_owned()));
            Some(match entity {
                Some(entity) => (entity, Some(block)),
                None => (block, None),
            })
        })?;
        let context = CapabilityContext {
            position: Some(position),
            side,
            stack: None,
        };
        let capability = provider.provide(self, &context)?;
        if capability.kind() != kind {
            return None;
        }
        if kind == CapabilityKind::Energy {
            self.native_energy(self.dim.id(), [position.x, position.y, position.z], side?)?;
        }
        Some(capability)
    }

    pub(super) fn native_query_item_capability(
        &mut self,
        stack: &ItemStack,
        kind: CapabilityKind,
    ) -> Option<Capability> {
        let provider = provider_for(&PROVIDERS, kind, || {
            if !self.native_simulates() || stack.count == 0 {
                return None;
            }
            Some((CapabilityTarget::Item(stack.item.to_string()), None))
        })?;
        let context = CapabilityContext {
            position: None,
            side: None,
            stack: Some(stack.clone()),
        };
        let capability = provider.provide(self, &context)?;
        if capability.kind() != kind {
            return None;
        }
        if kind == CapabilityKind::Energy {
            self.native_item_energy(stack)?;
        }
        Some(capability)
    }

    pub(super) fn native_capability_energy(
        &mut self,
        position: BlockPos,
        side: Direction,
    ) -> Option<CapabilityEnergy> {
        self.native_query_capability(position, Some(side), CapabilityKind::Energy)?;
        let buffer = self.native_energy(self.dim.id(), [position.x, position.y, position.z], side)?;
        Some(CapabilityEnergy {
            amount: buffer.amount,
            capacity: buffer.capacity,
            max_insert: buffer.max_insert,
            max_extract: buffer.max_extract,
            can_insert: buffer.can_insert,
            can_extract: buffer.can_extract,
        })
    }

    pub(super) fn native_capability_energy_insert(
        &mut self,
        position: BlockPos,
        side: Direction,
        amount: i32,
        simulate: bool,
    ) -> i32 {
        if self
            .native_query_capability(position, Some(side), CapabilityKind::Energy)
            .is_none()
        {
            return 0;
        }
        self.native_energy_insert([position.x, position.y, position.z], side, amount, simulate)
    }

    pub(super) fn native_capability_energy_extract(
        &mut self,
        position: BlockPos,
        side: Direction,
        amount: i32,
        simulate: bool,
    ) -> i32 {
        if self
            .native_query_capability(position, Some(side), CapabilityKind::Energy)
            .is_none()
        {
            return 0;
        }
        let dimension = self.dim.id();
        self.native_energy_extract(
            dimension,
            [position.x, position.y, position.z],
            side,
            amount,
            simulate,
        )
    }

    pub(super) fn native_capability_item_energy(
        &mut self,
        stack: &ItemStack,
    ) -> Option<CapabilityEnergy> {
        self.native_query_item_capability(stack, CapabilityKind::Energy)?;
        let buffer = self.native_item_energy(stack)?;
        Some(CapabilityEnergy {
            amount: buffer.amount,
            capacity: buffer.capacity,
            max_insert: buffer.max_insert,
            max_extract: buffer.max_extract,
            can_insert: buffer.max_insert > 0,
            can_extract: buffer.max_extract > 0,
        })
    }

    pub(super) fn native_capability_item_energy_transfer(
        &mut self,
        stack: &ItemStack,
        amount: i32,
        extract: bool,
        simulate: bool,
    ) -> Option<(i32, ItemStack)> {
        self.native_query_item_capability(stack, CapabilityKind::Energy)?;
        let (moved, result) = if extract {
            self.native_item_energy_extract(stack, amount)?
        } else {
            self.native_item_energy_insert(stack, amount)?
        };
        Some((moved, if simulate { stack.clone() } else { result }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestHost;

    impl Host for TestHost {}

    struct Inventory(Mutex<Option<ItemStack>>);

    impl ItemHandler for Inventory {
        fn slots(&self) -> usize {
            1
        }
        fn get_stack(&self, slot: usize) -> Option<ItemStack> {
            if slot == 0 {
                self.0.lock().unwrap().clone()
            } else {
                None
            }
        }
        fn set_stack(&self, slot: usize, stack: Option<ItemStack>) {
            assert_eq!(slot, 0);
            *self.0.lock().unwrap() = stack;
        }
        fn stack_limit(&self, _stack: &ItemStack) -> u32 {
            64
        }
    }

    struct ItemProvider(Arc<Inventory>);

    impl CapabilityProvider for ItemProvider {
        fn provide(&self, _host: &mut dyn Host, context: &CapabilityContext) -> Option<Capability> {
            assert_eq!(context.side, Some(Direction::West));
            Some(Capability::Items(self.0.clone()))
        }
    }

    #[test]
    fn no_providers_means_no_lookup_or_world_work() {
        let registry = OnceLock::<Providers>::new();
        let touched = std::cell::Cell::new(false);
        let result = provider_for(&registry, CapabilityKind::Items, || {
            touched.set(true);
            Some((
                CapabilityTarget::Block("minecraft:chest".to_owned()),
                None,
            ))
        });
        assert!(result.is_none());
        assert!(!touched.get());
        assert!(Providers::new(Vec::new()).unwrap().0.is_empty());
    }

    #[test]
    fn hopper_offer_uses_handler_without_a_block_container() {
        let handler = Arc::new(Inventory(Mutex::new(None)));
        let registry = OnceLock::new();
        let providers = Providers::new(vec![NativeCapabilityProvider {
            target: CapabilityTarget::Block("test:machine".to_owned()),
            kind: CapabilityKind::Items,
            provider: Arc::new(ItemProvider(handler.clone())),
        }])
        .unwrap();
        assert!(registry.set(providers).is_ok());
        let provider = provider_for(&registry, CapabilityKind::Items, || {
            Some((CapabilityTarget::Block("test:machine".to_owned()), None))
        })
        .unwrap();
        let context = CapabilityContext {
            position: Some(BlockPos::new(2, 3, 4)),
            side: Some(Direction::West),
            stack: None,
        };
        let Some(Capability::Items(items)) = provider.provide(&mut TestHost, &context) else {
            panic!("item capability missing");
        };
        let item = ItemStack::new("test:gear", 1);
        assert!(insert_item(items.as_ref(), item.clone(), true).is_none());
        assert!(handler.get_stack(0).is_none());
        assert!(insert_item(items.as_ref(), item, false).is_none());
        assert_eq!(handler.get_stack(0).unwrap().item.as_ref(), "test:gear");
        assert_eq!(handler.extract(0, 1, false).unwrap().count, 1);
        assert!(handler.get_stack(0).is_none());
    }

    #[test]
    fn tanks_simulate_and_preserve_fluid_identity_and_capacity() {
        let tanks = FluidStorage::new([750, 500]);
        let oil = FluidStack {
            fluid: "test:oil".to_owned(),
            amount: 1000,
        };
        assert_eq!(tanks.fill(&oil, true), 1000);
        assert!(tanks.tank(0).unwrap().contents.is_none());
        assert_eq!(tanks.fill(&oil, false), 1000);
        assert_eq!(tanks.tank(1).unwrap().contents.unwrap().amount, 250);
        assert_eq!(tanks.drain("test:oil", 900, true).unwrap().amount, 900);
        assert_eq!(tanks.drain("test:oil", 1250, false).unwrap().amount, 1000);
        assert!(tanks.drain("test:oil", 1, false).is_none());
    }
}
