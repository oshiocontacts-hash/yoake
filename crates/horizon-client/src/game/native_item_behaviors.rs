use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

use horizon_client::mod_api::{
    BlockHit, BlockPos, BlockStateId, Direction, Hand, Host, ItemStack, NativeInteractionResult,
    UseAnimation,
};

use super::{Game, native_block_behaviors::valid_id};

pub struct NativeItemUse<'a> {
    pub stack: &'a ItemStack,
    pub hand: Hand,
    pub client_side: bool,
    pub hit: Option<BlockHit>,
    pub entity: Option<(u32, &'a str)>,
}

pub trait ItemBehavior: Send + Sync {
    fn use_item(&self, _host: &mut dyn Host, _context: &NativeItemUse<'_>) -> NativeInteractionResult {
        NativeInteractionResult::Pass
    }
    fn use_on(&self, _host: &mut dyn Host, _context: &NativeItemUse<'_>) -> NativeInteractionResult {
        NativeInteractionResult::Pass
    }
    fn use_on_entity(
        &self,
        _host: &mut dyn Host,
        _context: &NativeItemUse<'_>,
    ) -> NativeInteractionResult {
        NativeInteractionResult::Pass
    }
    fn use_duration(&self, _stack: &ItemStack) -> Option<i32> {
        None
    }
    fn use_animation(&self, _stack: &ItemStack) -> Option<UseAnimation> {
        None
    }
    fn finish_using(
        &self,
        _host: &mut dyn Host,
        _context: &NativeItemUse<'_>,
        _result: &mut Option<ItemStack>,
    ) {
    }
    fn inventory_tick(
        &self,
        _host: &mut dyn Host,
        _stack: &ItemStack,
        _slot: i32,
        _selected: bool,
        _client_side: bool,
    ) {
    }
    fn attack_hit(
        &self,
        _host: &mut dyn Host,
        _stack: &ItemStack,
        _entity: u32,
        _kind: &str,
        _client_side: bool,
    ) {
    }
    fn mine_block(
        &self,
        _host: &mut dyn Host,
        _stack: &ItemStack,
        _position: BlockPos,
        _state: BlockStateId,
        _client_side: bool,
    ) -> bool {
        false
    }
    fn can_repair(&self, _stack: &ItemStack, _material: &ItemStack) -> Option<bool> {
        None
    }
    fn bar(&self, _stack: &ItemStack) -> Option<Option<(i32, u32)>> {
        None
    }
}

pub struct NativeItemBehavior {
    pub id: String,
    pub behavior: Arc<dyn ItemBehavior>,
}

#[derive(Default)]
struct Behaviors(HashMap<String, Arc<dyn ItemBehavior>>);

impl Behaviors {
    fn new(entries: Vec<NativeItemBehavior>) -> Result<Self, String> {
        let mut behaviors = HashMap::new();
        for entry in entries {
            if !valid_id(&entry.id) {
                return Err(format!("invalid mod item behavior id: {}", entry.id));
            }
            if behaviors.insert(entry.id.clone(), entry.behavior).is_some() {
                return Err(format!("duplicate item behavior: {}", entry.id));
            }
        }
        Ok(Self(behaviors))
    }

    fn lookup(&self, stack: &ItemStack) -> Option<&Arc<dyn ItemBehavior>> {
        self.0.get(&*stack.item)
    }
}

static BEHAVIORS: OnceLock<Behaviors> = OnceLock::new();

pub(super) fn register() -> Result<(), String> {
    let mut entries = Vec::new();
    Game::native_item_behaviors(&mut entries);
    let behaviors = Behaviors::new(entries)?;
    if behaviors.0.is_empty() {
        return Ok(());
    }
    BEHAVIORS
        .set(behaviors)
        .map_err(|_| "item behaviors already registered")?;
    horizon_client::stations::ITEM_REPAIR
        .set(|stack, material| lookup(stack)?.can_repair(stack, material))
        .map_err(|_| "item repair behavior already registered")?;
    Ok(())
}

pub(super) fn enabled() -> bool {
    BEHAVIORS.get().is_some()
}

pub(super) fn lookup(stack: &ItemStack) -> Option<&'static Arc<dyn ItemBehavior>> {
    BEHAVIORS.get()?.lookup(stack)
}

pub(super) fn duration(stack: &ItemStack, duration: &mut i32) {
    if let Some(answer) = lookup(stack).and_then(|behavior| behavior.use_duration(stack)) {
        *duration = answer.max(0);
    }
}

pub(super) fn animation(stack: &ItemStack, animation: &mut UseAnimation) {
    if let Some(answer) = lookup(stack).and_then(|behavior| behavior.use_animation(stack)) {
        *animation = answer;
    }
}

pub(super) fn bar(stack: &ItemStack, bar: &mut Option<(i32, u32)>) {
    if let Some(answer) = lookup(stack).and_then(|behavior| behavior.bar(stack)) {
        *bar = answer.map(|(width, color)| (width.clamp(0, 13), color & 0xffffff));
    }
}

impl Game<'_> {
    pub(super) fn item_behavior_use(
        &mut self,
        stack: &ItemStack,
        hit: Option<&super::Hit>,
        on_block: bool,
    ) -> NativeInteractionResult {
        let Some(behavior) = lookup(stack) else {
            return NativeInteractionResult::Pass;
        };
        let hit = hit.map(|hit| BlockHit {
            pos: hit.pos,
            location: hit.point,
            face: Direction::ALL
                .into_iter()
                .find(|direction| direction.normal() == hit.face)
                .unwrap_or(Direction::Up),
        });
        let context = NativeItemUse {
            stack,
            hand: self.hand,
            client_side: !self.native_authoritative(),
            hit,
            entity: None,
        };
        if context.hit.is_some() {
            let result = behavior.use_on(self, &context);
            if result != NativeInteractionResult::Pass {
                return self.item_behavior_start_using(stack, result);
            }
        }
        if on_block {
            return NativeInteractionResult::Pass;
        }
        let result = behavior.use_item(self, &context);
        self.item_behavior_start_using(stack, result)
    }

    pub(super) fn item_behavior_entity(
        &mut self,
        stack: &ItemStack,
        entity: u32,
        kind: &str,
    ) -> NativeInteractionResult {
        let Some(behavior) = lookup(stack) else {
            return NativeInteractionResult::Pass;
        };
        let context = NativeItemUse {
            stack,
            hand: self.hand,
            client_side: !self.native_authoritative(),
            hit: None,
            entity: Some((entity, kind)),
        };
        let result = behavior.use_on_entity(self, &context);
        self.item_behavior_start_using(stack, result)
    }

    fn item_behavior_start_using(
        &mut self,
        stack: &ItemStack,
        result: NativeInteractionResult,
    ) -> NativeInteractionResult {
        if result == NativeInteractionResult::Consume && self.use_duration(stack) > 0 {
            self.start_using_item();
        }
        result
    }

    pub(super) fn item_behavior_finish(&mut self, stack: &ItemStack, result: &mut Option<ItemStack>) {
        let Some(behavior) = lookup(stack) else {
            return;
        };
        let context = NativeItemUse {
            stack,
            hand: self.hand,
            client_side: !self.native_authoritative(),
            hit: None,
            entity: None,
        };
        behavior.finish_using(self, &context, result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Battery(AtomicUsize);
    impl ItemBehavior for Battery {
        fn use_item(
            &self,
            _host: &mut dyn Host,
            _context: &NativeItemUse<'_>,
        ) -> NativeInteractionResult {
            self.0.fetch_add(1, Ordering::Relaxed);
            NativeInteractionResult::Consume
        }
        fn use_on(
            &self,
            _host: &mut dyn Host,
            context: &NativeItemUse<'_>,
        ) -> NativeInteractionResult {
            assert_eq!(context.hit.unwrap().pos, [1, 2, 3]);
            self.0.fetch_add(1, Ordering::Relaxed);
            NativeInteractionResult::Success
        }
        fn use_on_entity(
            &self,
            _host: &mut dyn Host,
            context: &NativeItemUse<'_>,
        ) -> NativeInteractionResult {
            assert_eq!(context.entity, Some((7, "test:entity")));
            self.0.fetch_add(1, Ordering::Relaxed);
            NativeInteractionResult::Fail
        }
        fn use_duration(&self, _stack: &ItemStack) -> Option<i32> {
            Some(16)
        }
        fn use_animation(&self, _stack: &ItemStack) -> Option<UseAnimation> {
            Some(UseAnimation::Drink)
        }
        fn finish_using(
            &self,
            _host: &mut dyn Host,
            context: &NativeItemUse<'_>,
            result: &mut Option<ItemStack>,
        ) {
            *result = Some(ItemStack::new("test:discharged", context.stack.count));
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn inventory_tick(
            &self,
            _host: &mut dyn Host,
            _stack: &ItemStack,
            slot: i32,
            selected: bool,
            _client_side: bool,
        ) {
            assert_eq!(slot, 2);
            assert!(selected);
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn attack_hit(
            &self,
            _host: &mut dyn Host,
            _stack: &ItemStack,
            entity: u32,
            kind: &str,
            _client_side: bool,
        ) {
            assert_eq!((entity, kind), (7, "test:entity"));
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn mine_block(
            &self,
            _host: &mut dyn Host,
            _stack: &ItemStack,
            position: BlockPos,
            state: BlockStateId,
            _client_side: bool,
        ) -> bool {
            assert_eq!(position, BlockPos::new(1, 2, 3));
            assert_eq!(state, BlockStateId(4));
            self.0.fetch_add(1, Ordering::Relaxed);
            true
        }
        fn can_repair(&self, _stack: &ItemStack, material: &ItemStack) -> Option<bool> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Some(&*material.item == "test:cell")
        }
        fn bar(&self, _stack: &ItemStack) -> Option<Option<(i32, u32)>> {
            Some(Some((7, 0x33cc44)))
        }
    }

    #[test]
    fn empty_registry_does_not_touch_a_stack() {
        let behaviors = Behaviors::new(Vec::new()).unwrap();
        assert!(behaviors.0.is_empty());
        let stack = ItemStack::new("minecraft:stone", 1);
        let original = stack.clone();
        assert!(behaviors.lookup(&stack).is_none());
        assert!(lookup(&stack).is_none());
        assert!(!enabled());
        let mut original_duration = 32;
        let mut original_animation = UseAnimation::Drink;
        let mut original_bar = Some((5, 0x123456));
        duration(&stack, &mut original_duration);
        animation(&stack, &mut original_animation);
        bar(&stack, &mut original_bar);
        assert_eq!(original_duration, 32);
        assert_eq!(original_animation, UseAnimation::Drink);
        assert_eq!(original_bar, Some((5, 0x123456)));
        assert_eq!(stack, original);
    }

    #[test]
    fn shared_item_object_controls_duration_animation_repair_and_bar() {
        let battery = Arc::new(Battery(AtomicUsize::new(0)));
        let behaviors = Behaviors::new(vec![NativeItemBehavior {
            id: "test:battery".into(),
            behavior: battery.clone(),
        }])
        .unwrap();
        let stack = ItemStack::new("test:battery", 1);
        let behavior = behaviors.lookup(&stack).unwrap();
        assert_eq!(behavior.use_duration(&stack), Some(16));
        assert_eq!(behavior.use_animation(&stack), Some(UseAnimation::Drink));
        assert_eq!(
            behavior.can_repair(&stack, &ItemStack::new("test:cell", 1)),
            Some(true),
        );
        assert_eq!(
            behavior.can_repair(&stack, &ItemStack::new("minecraft:iron_ingot", 1)),
            Some(false),
        );
        assert_eq!(behavior.bar(&stack), Some(Some((7, 0x33cc44))));
        assert_eq!(battery.0.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn item_callbacks_share_the_object_on_both_sides_and_hands() {
        struct TestHost;
        impl Host for TestHost {}

        let battery = Arc::new(Battery(AtomicUsize::new(0)));
        let behaviors = Behaviors::new(vec![NativeItemBehavior {
            id: "test:battery".into(),
            behavior: battery.clone(),
        }])
        .unwrap();
        let stack = ItemStack::new("test:battery", 1);
        let behavior = behaviors.lookup(&stack).unwrap();
        for client_side in [false, true] {
            for hand in Hand::BOTH {
                let context = NativeItemUse {
                    stack: &stack,
                    hand,
                    client_side,
                    hit: Some(BlockHit {
                        pos: [1, 2, 3],
                        location: [1.5, 3.0, 3.5],
                        face: Direction::Up,
                    }),
                    entity: Some((7, "test:entity")),
                };
                assert_eq!(
                    behavior.use_item(&mut TestHost, &context),
                    NativeInteractionResult::Consume,
                );
                assert_eq!(
                    behavior.use_on(&mut TestHost, &context),
                    NativeInteractionResult::Success,
                );
                assert_eq!(
                    behavior.use_on_entity(&mut TestHost, &context),
                    NativeInteractionResult::Fail,
                );
                let mut result = Some(stack.clone());
                behavior.finish_using(&mut TestHost, &context, &mut result);
                assert_eq!(result, Some(ItemStack::new("test:discharged", 1)));
            }
            behavior.inventory_tick(&mut TestHost, &stack, 2, true, client_side);
            behavior.attack_hit(&mut TestHost, &stack, 7, "test:entity", client_side);
            assert!(behavior.mine_block(
                &mut TestHost,
                &stack,
                BlockPos::new(1, 2, 3),
                BlockStateId(4),
                client_side,
            ));
        }
        assert_eq!(battery.0.load(Ordering::Relaxed), 22);
    }

    #[test]
    fn vanilla_and_duplicate_claims_are_rejected() {
        let behavior = Arc::new(Battery(AtomicUsize::new(0)));
        assert!(
            Behaviors::new(vec![NativeItemBehavior {
                id: "minecraft:stick".into(),
                behavior: behavior.clone(),
            }])
            .is_err()
        );
        assert!(
            Behaviors::new(vec![
                NativeItemBehavior {
                    id: "test:cell".into(),
                    behavior: behavior.clone(),
                },
                NativeItemBehavior {
                    id: "test:cell".into(),
                    behavior,
                },
            ])
            .is_err()
        );
    }
}
