use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use horizon_client::mod_api::{
    BlockHit, BlockId, BlockPos, BlockShapeKind, BlockStateId, CollisionContext, CollisionEntity,
    CollisionItem, Direction, EntityIdentity, FluidState, Hand, Host, ItemStack, LevelHost,
    LevelWriteHost, NativeLootDrops, PlacementContext, VoxelShape,
};
use horizon_world::{
    level::Ticks,
    native_block_shapes::{self, NativeBlockShapes},
    registry::BlockRegistry,
};

use super::{Game, GameLevel, mod_api_host::LevelTicks};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NativeInteractionResult {
    #[default]
    Pass,
    Success,
    Consume,
    Fail,
}

pub struct NativeBlockUse<'a> {
    pub state: BlockStateId,
    pub hit: BlockHit,
    pub hand: Hand,
    pub stack: Option<&'a ItemStack>,
    pub client_side: bool,
}

pub struct NativeBlockContact<'a> {
    pub dimension: &'a str,
    pub client_side: bool,
    pub entity: i32,
    pub kind: &'a str,
    pub state: BlockStateId,
    pub position: BlockPos,
}

pub trait BlockBehavior: Send + Sync {
    fn randomly_ticking(&self) -> bool {
        false
    }
    fn use_block(&self, _host: &mut dyn Host, _context: &NativeBlockUse<'_>) -> NativeInteractionResult {
        NativeInteractionResult::Pass
    }
    fn attack(&self, _host: &mut dyn Host, _position: BlockPos, _state: BlockStateId) {}
    fn neighbor_changed(
        &self,
        _level: &mut dyn LevelWriteHost,
        _position: BlockPos,
        _state: BlockStateId,
        _changed: BlockId,
        _moved: bool,
    ) -> bool {
        false
    }
    fn random_tick(
        &self,
        _level: &mut dyn LevelWriteHost,
        _position: BlockPos,
        _state: BlockStateId,
    ) -> bool {
        false
    }
    fn scheduled_tick(
        &self,
        _level: &mut dyn LevelWriteHost,
        _position: BlockPos,
        _state: BlockStateId,
    ) -> bool {
        false
    }
    fn entity_inside(&self, _context: &NativeBlockContact<'_>) {}
    fn step_on(&self, _context: &NativeBlockContact<'_>) {}
    fn placement_state(
        &self,
        _level: &dyn LevelHost,
        _state: BlockStateId,
        _context: &PlacementContext,
        _fluid: FluidState,
    ) -> Option<Option<BlockStateId>> {
        None
    }
    fn shape(
        &self,
        _state: BlockStateId,
        _position: BlockPos,
        _context: &CollisionContext<'_>,
        _kind: BlockShapeKind,
    ) -> Option<VoxelShape> {
        None
    }
    fn is_signal_source(&self, _level: &dyn LevelHost, _state: BlockStateId) -> Option<bool> {
        None
    }
    fn signal(
        &self,
        _level: &dyn LevelHost,
        _position: BlockPos,
        _state: BlockStateId,
        _face: Direction,
        _strong: bool,
    ) -> Option<i32> {
        None
    }
    fn has_comparator_output(&self, _level: &dyn LevelHost, _state: BlockStateId) -> Option<bool> {
        None
    }
    fn comparator_output(
        &self,
        _level: &dyn LevelHost,
        _position: BlockPos,
        _state: BlockStateId,
        _face: Direction,
    ) -> Option<i32> {
        None
    }
    fn drops(&self, _host: &mut dyn Host, _context: &NativeLootDrops<'_>) -> Option<Vec<ItemStack>> {
        None
    }
}

pub struct NativeBlockBehavior {
    pub id: String,
    pub behavior: Arc<dyn BlockBehavior>,
}

#[derive(Default)]
struct Behaviors(HashMap<String, Arc<dyn BlockBehavior>>);

impl Behaviors {
    fn new(entries: Vec<NativeBlockBehavior>) -> Result<Self, String> {
        let mut behaviors = HashMap::new();
        for entry in entries {
            if !valid_id(&entry.id) {
                return Err(format!("invalid mod block behavior id: {}", entry.id));
            }
            if behaviors.insert(entry.id.clone(), entry.behavior).is_some() {
                return Err(format!("duplicate block behavior: {}", entry.id));
            }
        }
        Ok(Self(behaviors))
    }

    fn lookup(&self, registry: &BlockRegistry, state: BlockStateId) -> Option<&Arc<dyn BlockBehavior>> {
        self.0.get(&*registry.state(state)?.name)
    }
}

pub(super) fn valid_id(id: &str) -> bool {
    let Some((namespace, path)) = id.split_once(':') else {
        return false;
    };
    !namespace.is_empty()
        && namespace != "minecraft"
        && !path.is_empty()
        && namespace.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.-".contains(&byte)
        })
        && path.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"/_.-".contains(&byte)
        })
}

static BEHAVIORS: OnceLock<Behaviors> = OnceLock::new();

pub(super) fn enabled() -> bool {
    BEHAVIORS.get().is_some()
}

pub(super) fn register() -> Result<(), String> {
    let mut entries = Vec::new();
    Game::native_block_behaviors(&mut entries);
    let behaviors = Behaviors::new(entries)?;
    if behaviors.0.is_empty() {
        return Ok(());
    }
    BEHAVIORS
        .set(behaviors)
        .map_err(|_| "block behaviors already registered")?;
    native_block_shapes::install(NativeBlockShapes {
        claims: |name| named(name).is_some(),
        random_ticks: |name| named(name).is_some_and(|behavior| behavior.randomly_ticking()),
        shape: |registry, state, position, context, kind| {
            lookup(registry, state)?.shape(state, position, context, kind)
        },
    })
    .map_err(|_| "block shapes already registered")?;
    horizon_client::block_contact::BEHAVIOR_HOOK
        .set(contact)
        .map_err(|_| "block contacts already registered")?;
    Ok(())
}

pub(super) fn named(name: &str) -> Option<&'static Arc<dyn BlockBehavior>> {
    BEHAVIORS.get()?.0.get(name)
}

pub(super) fn lookup(
    registry: &BlockRegistry,
    state: BlockStateId,
) -> Option<&'static Arc<dyn BlockBehavior>> {
    BEHAVIORS.get()?.lookup(registry, state)
}

impl GameLevel<'_> {
    pub(super) fn behavior_tick(
        &mut self,
        ticks: &mut Ticks,
        position: BlockPos,
        state: BlockStateId,
        random: bool,
    ) -> bool {
        let Some(behavior) = lookup(self.registry, state) else {
            return false;
        };
        let mut host = LevelTicks { level: self, ticks };
        if random {
            behavior.random_tick(&mut host, position, state)
        } else {
            behavior.scheduled_tick(&mut host, position, state)
        }
    }
}

fn contact(dimension: &str, client_side: bool, event: horizon_client::block_contact::Event<'_>) {
    use horizon_client::block_contact::Event;
    let (contact, stepped) = match event {
        Event::StepOn(contact) => (contact, true),
        Event::Inside(contact) => (contact, false),
        Event::FallOn(_, _) => return,
    };
    let Some(behavior) = named(contact.block) else {
        return;
    };
    let context = NativeBlockContact {
        dimension,
        client_side,
        entity: contact.entity,
        kind: contact.kind,
        state: contact.state,
        position: contact.position,
    };
    if stepped {
        behavior.step_on(&context);
    } else {
        behavior.entity_inside(&context);
    }
}

impl Game<'_> {
    pub(super) fn block_behavior_outline(
        &self,
        position: [i32; 3],
        state: BlockStateId,
    ) -> Option<Vec<[f64; 6]>> {
        let behavior = lookup(self.pack.assets.registry, state)?;
        let context = self.player.as_ref().map_or_else(
            || CollisionContext::empty(false),
            |player| {
                let main_hand = self.inventory[self.selected].as_ref().map_or(
                    CollisionItem::EMPTY,
                    |stack| CollisionItem {
                        item: &stack.item,
                        count: i32::try_from(stack.count).unwrap_or(i32::MAX),
                    },
                );
                CollisionContext::for_entity(
                    CollisionEntity {
                        identity: EntityIdentity(0),
                        y: player.position.y,
                        descending: self.keys.contains(&winit::keyboard::KeyCode::ShiftLeft),
                        main_hand: Some(main_hand),
                    },
                    false,
                )
            },
        );
        let shape = behavior.shape(
            state,
            BlockPos::new(position[0], position[1], position[2]),
            &context,
            BlockShapeKind::Outline,
        )?;
        Some(
            shape
                .boxes()
                .iter()
                .map(|bounds| {
                    let minimum = bounds.min_position();
                    let maximum = bounds.max_position();
                    [
                        minimum.x + f64::from(position[0]),
                        minimum.y + f64::from(position[1]),
                        minimum.z + f64::from(position[2]),
                        maximum.x + f64::from(position[0]),
                        maximum.y + f64::from(position[1]),
                        maximum.z + f64::from(position[2]),
                    ]
                })
                .collect(),
        )
    }

    pub(super) fn behavior_interaction(&mut self, result: NativeInteractionResult) -> bool {
        match result {
            NativeInteractionResult::Pass => false,
            NativeInteractionResult::Success => {
                self.swing_arm();
                true
            }
            NativeInteractionResult::Consume => true,
            NativeInteractionResult::Fail => {
                self.use_failed = true;
                true
            }
        }
    }

    pub(super) fn block_behavior_use(
        &mut self,
        position: [i32; 3],
        point: [f64; 3],
        face: [i32; 3],
    ) -> NativeInteractionResult {
        if BEHAVIORS.get().is_none() {
            return NativeInteractionResult::Pass;
        }
        let state = self.world.block(position[0], position[1], position[2]);
        let Some(behavior) = lookup(self.pack.assets.registry, state) else {
            return NativeInteractionResult::Pass;
        };
        let held = self.held().cloned();
        let context = NativeBlockUse {
            state,
            hit: BlockHit {
                pos: position,
                location: point,
                face: Direction::ALL
                    .into_iter()
                    .find(|direction| direction.normal() == face)
                    .unwrap_or(Direction::Up),
            },
            hand: self.hand,
            stack: held.as_ref(),
            client_side: !self.native_authoritative(),
        };
        behavior.use_block(self, &context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::mod_api::{
        AttachmentError, Attachments, Holder, LegacyRandom, LevelRandomHost, LevelTicksHost,
        ParticleOptions,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn registry() -> BlockRegistry {
        BlockRegistry::from_facts_json(
            &br#"{"defaults":{"minecraft:air":0,"test:machine":1},"states":[
                ["minecraft:air",true,false,"air","minecraft:air"],
                ["test:machine",false,false,"off",{"id":"test:machine","properties":{"power":"0"}}],
                ["test:machine",false,false,"on",{"id":"test:machine","properties":{"power":"12"}}]
            ]}"#[..],
        )
        .unwrap()
    }

    #[derive(Default)]
    struct TestLevel {
        state: u16,
        attachments: Attachments,
        scheduled: Vec<([i32; 3], i32)>,
    }

    impl LevelHost for TestLevel {
        fn block_state(&self, _pos: [i32; 3]) -> BlockStateId {
            BlockStateId(self.state.into())
        }
        fn is_client_side(&self) -> bool {
            false
        }
        fn state_with_property(
            &self,
            _state: BlockStateId,
            _key: &str,
            value: &str,
        ) -> Option<BlockStateId> {
            value.parse::<u16>().ok().map(|state| BlockStateId(state.into()))
        }
    }

    impl LevelRandomHost for TestLevel {
        fn attachments(&mut self, _holder: Holder) -> Result<&mut Attachments, AttachmentError> {
            Ok(&mut self.attachments)
        }
        fn random(&mut self) -> Option<&mut LegacyRandom> {
            None
        }
    }

    impl LevelTicksHost for TestLevel {
        fn schedule_block_tick(&mut self, pos: [i32; 3], _block: &str, delay: i32) -> bool {
            self.scheduled.push((pos, delay));
            true
        }
        fn schedule_fluid_tick(&mut self, _pos: [i32; 3], _fluid: &str, _delay: i32) -> bool {
            false
        }
    }

    impl LevelWriteHost for TestLevel {
        fn set_block_property(&mut self, _pos: [i32; 3], _key: &str, value: &str, _flags: i32) -> bool {
            let Ok(state) = value.parse() else {
                return false;
            };
            self.state = state;
            true
        }
        fn play_sound(
            &mut self,
            _sound: &str,
            _at: [f64; 3],
            _source: usize,
            _volume: f32,
            _pitch: f32,
        ) -> bool {
            false
        }
        fn send_particles(
            &mut self,
            _options: &ParticleOptions,
            _state: Option<BlockStateId>,
            _at: [f64; 3],
            _spread: [f64; 3],
            _speed: f64,
            _count: i32,
        ) -> bool {
            false
        }
    }

    struct Machine;
    impl BlockBehavior for Machine {
        fn randomly_ticking(&self) -> bool {
            true
        }
        fn neighbor_changed(
            &self,
            level: &mut dyn LevelWriteHost,
            position: BlockPos,
            _state: BlockStateId,
            _changed: BlockId,
            _moved: bool,
        ) -> bool {
            level.set_block_property([position.x, position.y, position.z], "power", "3", 3)
        }
        fn random_tick(
            &self,
            level: &mut dyn LevelWriteHost,
            position: BlockPos,
            _state: BlockStateId,
        ) -> bool {
            level.schedule_block_tick([position.x, position.y, position.z], "test:machine", 4)
        }
        fn scheduled_tick(
            &self,
            level: &mut dyn LevelWriteHost,
            position: BlockPos,
            _state: BlockStateId,
        ) -> bool {
            level.set_block_property([position.x, position.y, position.z], "power", "12", 3)
        }
        fn is_signal_source(&self, _level: &dyn LevelHost, _state: BlockStateId) -> Option<bool> {
            Some(true)
        }
        fn signal(
            &self,
            level: &dyn LevelHost,
            _position: BlockPos,
            _state: BlockStateId,
            _face: Direction,
            _strong: bool,
        ) -> Option<i32> {
            Some(level.block_state([0, 0, 0]).0 as i32)
        }
        fn has_comparator_output(&self, _level: &dyn LevelHost, _state: BlockStateId) -> Option<bool> {
            Some(true)
        }
        fn placement_state(
            &self,
            level: &dyn LevelHost,
            state: BlockStateId,
            context: &PlacementContext,
            _fluid: FluidState,
        ) -> Option<Option<BlockStateId>> {
            Some(if context.sneaking {
                None
            } else {
                level.state_with_property(state, "power", "2")
            })
        }
        fn drops(&self, _host: &mut dyn Host, context: &NativeLootDrops<'_>) -> Option<Vec<ItemStack>> {
            assert_eq!(context.block, Some(([0, 0, 0], BlockStateId(2))));
            Some(vec![ItemStack::new("test:machine", 1)])
        }
        fn comparator_output(
            &self,
            level: &dyn LevelHost,
            _position: BlockPos,
            _state: BlockStateId,
            _face: Direction,
        ) -> Option<i32> {
            Some(level.block_state([0, 0, 0]).0 as i32 / 2)
        }
    }

    #[test]
    fn machine_ticks_write_the_same_level_read_by_signals() {
        let mut level = TestLevel::default();
        let registry = registry();
        let behaviors = Behaviors::new(vec![NativeBlockBehavior {
            id: "test:machine".into(),
            behavior: Arc::new(Machine),
        }])
        .unwrap();
        let behavior = behaviors.lookup(&registry, BlockStateId(1)).unwrap();
        let position = BlockPos::new(0, 0, 0);
        assert!(behavior.randomly_ticking());
        assert!(behavior.neighbor_changed(&mut level, position, BlockStateId(1), BlockId(0), false));
        assert_eq!(level.state, 3);
        assert!(behavior.random_tick(&mut level, position, BlockStateId(1)));
        assert_eq!(level.scheduled, vec![([0, 0, 0], 4)]);
        assert!(behavior.scheduled_tick(&mut level, position, BlockStateId(1)));
        assert_eq!(behavior.is_signal_source(&level, BlockStateId(2)), Some(true));
        for strong in [false, true] {
            assert_eq!(
                behavior.signal(&level, position, BlockStateId(2), Direction::Up, strong),
                Some(12),
            );
        }
        assert_eq!(
            behavior.has_comparator_output(&level, BlockStateId(2)),
            Some(true),
        );
        assert_eq!(
            behavior.comparator_output(&level, position, BlockStateId(2), Direction::Up),
            Some(6),
        );
    }

    #[test]
    fn placement_can_select_or_refuse_a_state_and_drops_replace_loot() {
        struct TestHost;
        impl Host for TestHost {}

        let mut context = PlacementContext {
            pos: BlockPos::new(0, 0, 0),
            hit_pos: BlockPos::new(0, -1, 0),
            face: Direction::Up,
            hit: horizon_core::geometry::Vec3::new(0.5, 0.0, 0.5),
            inside: false,
            yaw: 90.0,
            pitch: 0.0,
            sneaking: false,
            item: "test:machine".into(),
            replace_clicked: false,
        };
        let level = TestLevel::default();
        assert_eq!(
            Machine.placement_state(&level, BlockStateId(1), &context, FluidState::EMPTY),
            Some(Some(BlockStateId(2))),
        );
        context.sneaking = true;
        assert_eq!(
            Machine.placement_state(&level, BlockStateId(1), &context, FluidState::EMPTY),
            Some(None),
        );
        let drops = NativeLootDrops {
            table: "test:blocks/machine",
            dimension: "test:dimension",
            origin: [0.5; 3],
            block: Some(([0, 0, 0], BlockStateId(2))),
            entity: None,
            player: Some(7),
            tool: None,
            explosion: None,
            used: false,
        };
        assert_eq!(
            Machine.drops(&mut TestHost, &drops),
            Some(vec![ItemStack::new("test:machine", 1)]),
        );
    }

    struct Counter(AtomicUsize);
    impl BlockBehavior for Counter {
        fn use_block(
            &self,
            _host: &mut dyn Host,
            context: &NativeBlockUse<'_>,
        ) -> NativeInteractionResult {
            assert_eq!(context.stack.unwrap().item.as_ref(), "test:tool");
            assert_eq!(context.hit.face, Direction::Up);
            self.0.fetch_add(1, Ordering::Relaxed);
            NativeInteractionResult::Success
        }
        fn attack(&self, _host: &mut dyn Host, _position: BlockPos, _state: BlockStateId) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn entity_inside(&self, _context: &NativeBlockContact<'_>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn step_on(&self, _context: &NativeBlockContact<'_>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn shape(
            &self,
            _state: BlockStateId,
            _position: BlockPos,
            _context: &CollisionContext<'_>,
            _kind: BlockShapeKind,
        ) -> Option<VoxelShape> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Some(VoxelShape::empty())
        }
    }

    #[test]
    fn empty_registry_has_no_callbacks() {
        let behaviors = Behaviors::new(Vec::new()).unwrap();
        assert!(behaviors.0.is_empty());
        let registry = registry();
        assert!(behaviors.lookup(&registry, BlockStateId(1)).is_none());
        assert!(lookup(&registry, BlockStateId(u32::MAX)).is_none());
        assert!(!enabled());
        assert!(!valid_id("minecraft:stone"));
    }

    #[test]
    fn one_object_is_shared_by_all_block_states() {
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        let behaviors = Behaviors::new(vec![NativeBlockBehavior {
            id: "test:machine".into(),
            behavior: counter.clone(),
        }])
        .unwrap();
        let registry = registry();
        assert!(behaviors.lookup(&registry, BlockStateId(0)).is_none());
        for state in [BlockStateId(1), BlockStateId(2)] {
            let behavior = behaviors.lookup(&registry, state).unwrap();
            for kind in [BlockShapeKind::Outline, BlockShapeKind::Collision] {
                assert!(
                    behavior
                        .shape(
                            state,
                            BlockPos::new(0, 0, 0),
                            &CollisionContext::empty(false),
                            kind,
                        )
                        .unwrap()
                        .is_empty()
                );
            }
        }
        assert_eq!(counter.0.load(Ordering::Relaxed), 4);
        assert!(
            Behaviors::new(vec![NativeBlockBehavior {
                id: "minecraft:stone".into(),
                behavior: counter,
            }])
            .is_err()
        );
    }

    #[test]
    fn use_attack_and_contacts_receive_both_logical_sides() {
        struct TestHost;
        impl Host for TestHost {}

        let counter = Counter(AtomicUsize::new(0));
        let stack = ItemStack::new("test:tool", 1);
        let position = BlockPos::new(1, 2, 3);
        for client_side in [false, true] {
            for hand in [Hand::Main, Hand::Off] {
                let context = NativeBlockUse {
                    state: BlockStateId(1),
                    hit: BlockHit {
                        pos: [1, 2, 3],
                        location: [1.5, 3.0, 3.5],
                        face: Direction::Up,
                    },
                    hand,
                    stack: Some(&stack),
                    client_side,
                };
                assert_eq!(
                    counter.use_block(&mut TestHost, &context),
                    NativeInteractionResult::Success,
                );
            }
            counter.attack(&mut TestHost, position, BlockStateId(1));
            let context = NativeBlockContact {
                dimension: "test:dimension",
                client_side,
                entity: 7,
                kind: "test:entity",
                state: BlockStateId(1),
                position,
            };
            counter.entity_inside(&context);
            counter.step_on(&context);
        }
        assert_eq!(counter.0.load(Ordering::Relaxed), 10);
    }
}
