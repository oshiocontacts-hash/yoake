#[cfg(test)]
mod crash;

#[cfg(test)]
mod tests {
    use crate::crash::mod_guard::{self, CallOutcome as GuardOutcome};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[derive(Clone)]
    struct ItemStack;
    struct StackView {
        count: i32,
    }
    struct BlockEntityDraw(Vec<u32>);
    #[allow(dead_code)]
    mod stubs {
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub(super) struct BlockStateId;
        #[derive(Clone, Copy)]
        pub(super) struct FluidState;
        pub(super) struct PlacementContext;
        #[derive(Clone, Copy)]
        pub(super) struct BlockPos;
        #[derive(Clone, Copy)]
        pub(super) struct BlockId;
        #[derive(Clone, Copy)]
        pub(super) struct Direction;
        #[derive(Clone, Copy)]
        pub(super) struct LevelDimension;
        pub(super) struct Ticks;
        pub(super) trait LevelHost {}
        pub(super) trait LevelTicksHost: LevelHost {}
        pub(super) trait LevelRandomHost: LevelHost {}
        pub(super) trait LevelWriteHost: LevelTicksHost + LevelRandomHost {}
        #[derive(Clone)]
        pub(super) struct Compound;
        pub(super) type NativeDispensed = (Option<super::ItemStack>, i32);
        pub(super) struct NativeLootDrops<'a>(pub &'a ());
        pub(super) struct NativeNoteBlockChange<'a>(pub &'a ());
        pub(super) struct NameTagLine;
        pub(super) struct MouseButtons;
        pub(super) struct NativeScreenView<'a>(pub &'a ());
        #[derive(Clone, Copy, Debug, PartialEq)]
        pub(super) struct NativeInventoryLayout {
            pub offset: [f32; 2],
            pub width: u32,
            pub show_crafting: bool,
        }
        pub(super) struct Corners;
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
        pub(super) struct ItemEnergyBuffer;
        #[derive(Clone)]
        pub(super) struct Suggestion;
        pub(super) struct HudBatch;
        #[derive(Clone)]
        pub(super) struct BiomeColors;
        #[derive(Clone)]
        pub(super) struct SpawnEntry;
        pub(super) struct UseAnimation;
        #[derive(Clone, Copy)]
        pub(super) struct Hand;
        pub(super) struct NativeMobRender<'a>(pub &'a ());
        #[derive(Clone)]
        pub(super) struct NativeShake;
        pub(super) struct Music;
        pub(super) struct MusicLayer;
        #[derive(Clone)]
        pub(super) struct NativeBlockRim;
        #[derive(Clone)]
        pub(super) struct NativeColoredLight;
        pub(super) struct NativeMinimap;
        pub(super) struct NativeMod;
        pub(super) struct NativePanelEvent;
        pub(super) struct NativeScreen;
        pub(super) struct ModBlockEntity;
        #[derive(Clone)]
        pub(super) struct ModContainer;
        #[derive(Clone)]
        pub(super) struct MerchantOffer;
        #[derive(Clone)]
        pub(super) struct ModFluidTanks;
        #[derive(Clone)]
        pub(super) struct ModEnergyBuffer;
        #[derive(Clone)]
        pub(super) struct LootStack;
        pub(super) struct NativeLootContext<'a>(pub &'a ());
        pub(super) struct NativeDataMapType;
        pub(super) struct RecipeBookCategory;
        #[derive(Clone)]
        pub(super) struct NativeMapDecoration;
        pub(super) struct NativeMapInfo<'a>(pub &'a ());
        pub(super) struct NativePlayerRender<'a>(pub &'a ());
        pub(super) struct NativeCommand;
        #[derive(Clone, Debug, Default, PartialEq, Eq)]
        pub(super) enum NativeChatRecipients {
            #[default]
            All,
            Only(Vec<u128>),
        }
        #[derive(Clone, Debug, PartialEq)]
        pub(super) struct NativeCrosshairEntity {
            pub entity_id: u32,
            pub uuid: Option<u128>,
            pub kind: String,
            pub name: String,
            pub distance: f32,
        }
        pub(super) struct NativeLifecycleContext;
        pub(super) struct NativeServerPlayer;
        #[derive(Clone, Copy)]
        pub(super) struct ServerTickPhase;
        #[derive(Clone, Copy)]
        pub(super) struct WorldPhase;
    }
    use serde_json::{Map, Value};
    use stubs::*;
    #[derive(Clone)]
    struct Text;
    #[derive(Debug, PartialEq)]
    struct NativeZoom(u32);
    enum Flow {
        Continue,
        Cancel,
    }
    trait Host {}
    trait RenderHooks {}
    struct TestHost;
    impl Host for TestHost {}
    mod winit {
        pub mod event {
            pub struct KeyEvent;
        }
    }
    #[allow(dead_code)]
    mod generated {
        mod horizon_mods {
            pub mod content {
                #[derive(Clone)]
                pub struct Block;
                #[derive(Clone)]
                pub struct Item;
            }
        }
        mod horizon_client {
            pub mod camera {
                pub struct CameraEntity;
                pub struct FovPlayer;
            }
            pub mod menu {
                #[derive(Clone)]
                pub struct AnvilCraft;
            }
            pub mod fog {
                #[derive(Clone, Copy)]
                pub struct FogType;
                #[derive(Clone)]
                pub struct FogData;
            }
            pub mod first_person {
                #[derive(Clone, Copy)]
                pub struct Hand;
            }
            pub mod hud_layout {
                #[derive(Clone)]
                pub struct HealthInputs;
                #[derive(Clone)]
                pub struct GuiDraw;
            }
        }
        include!(concat!(env!("OUT_DIR"), "/mod_api_generated.rs"));
    }
    use generated::*;
    include!("server_chat_tests.rs");
    include!("pvp_attacker_tests.rs");
    thread_local! {
        static GUARD: std::cell::Cell<Option<&'static mod_guard::Registry>> = std::cell::Cell::new(
            Some(Box::leak(Box::new(mod_guard::Registry::new(
                &[],
                16,
                GUARDED_HOOKS,
                mod_guard::PanicMode::Contain,
            ))))
        );
    }

    fn guard_dispatch(count: usize, hook: &'static str) -> Option<mod_guard::Dispatch<'static>> {
        GUARD.with(|guard| match guard.get() {
            Some(registry) => registry.dispatch(count, hook),
            None => mod_guard::dispatch(count, hook),
        })
    }

    fn guard_index(index: usize) -> usize {
        index
    }

    fn reset_guard() {
        crate::crash::install_hook();
        GUARD.with(|guard| {
            guard.set(Some(Box::leak(Box::new(mod_guard::Registry::new(
                &[],
                16,
                GUARDED_HOOKS,
                mod_guard::PanicMode::Contain,
            )))));
        });
    }
    thread_local! {
        /// The hooks each dispatch entered, a mod at a time.
        static ENTERED: std::cell::RefCell<Vec<(usize, &'static str)>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    fn entered<'a>(mods: &'a [Box<dyn Hooks>], hook: &'static str) -> Entered<'a> {
        Entered {
            mods: mods.iter().enumerate(),
            hook,
            guard: None,
        }
    }

    struct Entered<'a> {
        mods: std::iter::Enumerate<std::slice::Iter<'a, Box<dyn Hooks>>>,
        hook: &'static str,
        guard: Option<crate::crash::NativeModGuard>,
    }

    impl<'a> Iterator for Entered<'a> {
        type Item = &'a Box<dyn Hooks>;

        fn next(&mut self) -> Option<Self::Item> {
            self.guard.take();
            let (index, hooks) = self.mods.next()?;
            ENTERED.with(|entered| entered.borrow_mut().push((index, self.hook)));
            self.guard = Some(crate::crash::enter_native_mod(
                crate::crash::NativeModCall {
                    id: None,
                    index,
                    hook: self.hook,
                },
            ));
            Some(hooks)
        }
    }
    struct Mod {
        id: u32,
        calls: Arc<Mutex<Vec<u32>>>,
    }
    impl Hooks for Mod {
        fn block_entity_submit(&self, _: &dyn Host, event: &mut BlockEntitySubmitEvent<'_>) {
            assert_eq!(event.pos, [1, 2, 3]);
            assert_eq!(event.block, "example:machine");
            event.draw.0.push(self.id);
        }
        fn living_entity_use_item_finish(
            &self,
            _: &mut dyn Host,
            event: &mut LivingEntityUseItemFinishEvent<'_>,
        ) {
            *event.result = None;
        }
        fn mouse_button_pre(&self, _: &mut dyn Host, _: &mut MouseButtonPreEvent) -> Flow {
            self.calls.lock().unwrap().push(self.id);
            if self.id == 1 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn menu_slot_rules(&self, event: &mut MenuSlotRulesEvent<'_>) {
            event.may_place = false;
            event.max_stack = self.id;
        }
        fn living_entity_use_item_tick(
            &self,
            _: &mut dyn Host,
            event: &mut LivingEntityUseItemTickEvent<'_>,
        ) -> Flow {
            event.duration -= 1;
            if self.id == 1 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn loot_table_load(&self, event: &mut LootTableLoadEvent<'_>) {
            event.table.as_array_mut().unwrap().push(self.id.into());
        }
        fn modify_default_components(&self, event: &mut ModifyDefaultComponentsEvent<'_>) {
            let value = event
                .components
                .entry("order")
                .or_insert_with(|| serde_json::json!([]));
            value.as_array_mut().unwrap().push(self.id.into());
        }
        fn level_tick_pre(&self, _: &mut dyn Host, _: &mut LevelTickPreEvent<'_>) {
            self.calls.lock().unwrap().push(self.id);
        }
        fn tooltip(&self, _: &dyn Host, event: &mut TooltipEvent<'_>) {
            event.lines.push((Text, self.id));
        }
        fn zoom(&self, _: &dyn Host, _: &mut ZoomEvent) -> Option<NativeZoom> {
            self.calls.lock().unwrap().push(self.id);
            Some(NativeZoom(self.id))
        }
        fn should_apply_effect_tick_this_tick(
            &self,
            _: &dyn Host,
            event: &mut ShouldApplyEffectTickThisTickEvent<'_>,
        ) {
            event.apply = !event.apply;
            self.calls.lock().unwrap().push(self.id);
        }
        fn living_incoming_damage(&self, event: &mut LivingIncomingDamageEvent<'_>) -> Flow {
            event.amount = event.amount * 2.0 + self.id as f32;
            if self.id == 2 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
        fn use_item_on(&self, _: &mut dyn Host, _: &mut UseItemOnEvent<'_>) -> Flow {
            self.calls.lock().unwrap().push(self.id);
            if self.id == 1 {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }
    struct ProbeMod {
        id: u32,
        panics: bool,
        calls: Arc<Mutex<Vec<(u32, &'static str)>>>,
    }

    impl ProbeMod {
        fn record(&self, hook: &'static str) {
            self.calls.lock().unwrap().push((self.id, hook));
            assert!(!self.panics, "broken probe");
        }
    }

    impl Hooks for ProbeMod {
        fn loot_table_load(&self, event: &mut LootTableLoadEvent<'_>) {
            event.table.as_array_mut().unwrap().push(self.id.into());
            self.record("loot_table_load");
        }

        fn living_incoming_damage(&self, event: &mut LivingIncomingDamageEvent<'_>) -> Flow {
            event.amount += self.id as f32;
            self.record("living_incoming_damage");
            Flow::Continue
        }

        fn zoom(&self, _: &dyn Host, _: &mut ZoomEvent) -> Option<NativeZoom> {
            self.record("zoom");
            Some(NativeZoom(self.id))
        }

        fn tooltip(&self, _: &dyn Host, event: &mut TooltipEvent<'_>) {
            event.lines.push((Text, self.id));
            self.record("tooltip");
        }

        fn level_tick_pre(&self, _: &mut dyn Host, _: &mut LevelTickPreEvent<'_>) {
            self.record("level_tick_pre");
        }
    }

    #[test]
    fn dispatch_without_a_registry_unwinds_and_keeps_the_mod_callable() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        struct PanicOnce {
            panics: AtomicBool,
            calls: Arc<AtomicUsize>,
        }

        impl Hooks for PanicOnce {
            fn loot_table_load(&self, event: &mut LootTableLoadEvent<'_>) {
                self.calls.fetch_add(1, Ordering::Relaxed);
                event.table.as_array_mut().unwrap().push(1.into());
                if self.panics.swap(false, Ordering::Relaxed) {
                    panic!("broken unguarded mod");
                }
            }
        }

        reset_guard();
        GUARD.with(|guard| guard.set(None));
        assert!(mod_guard::dispatch(0, "loot_table_load").is_none());
        crate::crash::record_panic_marks();
        let calls = Arc::new(AtomicUsize::new(0));
        let mods: Vec<Box<dyn Hooks>> = vec![Box::new(PanicOnce {
            panics: AtomicBool::new(true),
            calls: calls.clone(),
        })];
        let previous = crate::crash::NativeModCall {
            id: Some("outer"),
            index: 99,
            hook: "outer_hook",
        };
        let mark = crate::crash::enter_native_mod(previous);
        let mut table = serde_json::json!([]);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            dispatch_loot_table_load(&mods, "table", &mut table);
        }));
        let restored_mark = crate::crash::native_mod_call();
        drop(mark);
        let caller_result = std::panic::catch_unwind(|| panic!("unguarded caller"));
        let panic_marks = crate::crash::take_panic_marks();
        assert_eq!(
            result.unwrap_err().downcast_ref::<&str>(),
            Some(&"broken unguarded mod")
        );
        assert!(caller_result.is_err());
        assert_eq!(restored_mark, Some(previous));
        assert_eq!(
            panic_marks,
            [
                Some(crate::crash::NativeModCall {
                    id: None,
                    index: 0,
                    hook: "loot_table_load",
                }),
                None,
            ]
        );
        assert!(mod_guard::mod_statuses().is_empty());
        assert!(mod_guard::profile_snapshot().is_empty());
        assert!(!mod_guard::profile_enabled());
        dispatch_loot_table_load(&mods, "table", &mut table);
        assert_eq!(table, serde_json::json!([1, 1]));
        assert_eq!(calls.load(Ordering::Relaxed), 2);
        assert_eq!(crate::crash::native_mod_call(), None);
        reset_guard();
    }

    #[test]
    fn panic_keeps_buffer_changes_and_quarantines_all_hooks() {
        reset_guard();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods: Vec<Box<dyn Hooks>> = (1..=3)
            .map(|id| {
                Box::new(ProbeMod {
                    id,
                    panics: id == 2,
                    calls: calls.clone(),
                }) as Box<dyn Hooks>
            })
            .collect();
        let previous = crate::crash::NativeModCall {
            id: Some("outer"),
            index: 99,
            hook: "outer_hook",
        };
        let _mark = crate::crash::enter_native_mod(previous);
        let mut table = serde_json::json!([99]);
        dispatch_loot_table_load(&mods, "table", &mut table);
        assert_eq!(table, serde_json::json!([99, 1, 2]));
        assert_eq!(
            *calls.lock().unwrap(),
            [(1, "loot_table_load"), (2, "loot_table_load")]
        );
        assert_eq!(crate::crash::native_mod_call(), Some(previous));
        calls.lock().unwrap().clear();
        dispatch_loot_table_load(&mods, "table", &mut table);
        assert_eq!(table, serde_json::json!([99, 1, 2, 1, 3]));
        dispatch_level_tick_pre(&mut TestHost, &mods, "overworld");
        assert_eq!(
            *calls.lock().unwrap(),
            [
                (1, "loot_table_load"),
                (3, "loot_table_load"),
                (1, "level_tick_pre"),
                (3, "level_tick_pre"),
            ]
        );
        GUARD.with(|guard| {
            let statuses = guard.get().unwrap().statuses();
            assert!(!statuses[1].enabled);
            assert_eq!(
                statuses[1].first_error.as_ref().unwrap().hook,
                "loot_table_load"
            );
        });
    }

    #[test]
    fn cancel_panic_does_not_cancel_or_commit_scalars() {
        reset_guard();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods: Vec<Box<dyn Hooks>> = (1..=3)
            .map(|id| {
                Box::new(ProbeMod {
                    id,
                    panics: id == 2,
                    calls: calls.clone(),
                }) as Box<dyn Hooks>
            })
            .collect();
        let mut amount = 10.0;
        let mut canceled = false;
        dispatch_living_incoming_damage(
            &mods,
            0,
            "kind",
            "damage",
            0,
            0,
            &mut amount,
            &mut canceled,
        );
        assert_eq!(amount, 10.0);
        assert!(!canceled);
        assert_eq!(calls.lock().unwrap().len(), 2);
        dispatch_living_incoming_damage(
            &mods,
            0,
            "kind",
            "damage",
            0,
            0,
            &mut amount,
            &mut canceled,
        );
        assert_eq!(amount, 14.0);
        assert!(!canceled);
    }

    #[test]
    fn replace_panics_leave_none_and_collect_panics_keep_changes() {
        for collect in [false, true] {
            reset_guard();
            let calls = Arc::new(Mutex::new(Vec::new()));
            let mods: Vec<Box<dyn Hooks>> = [2, 3]
                .into_iter()
                .map(|id| {
                    Box::new(ProbeMod {
                        id,
                        panics: id == 2,
                        calls: calls.clone(),
                    }) as Box<dyn Hooks>
                })
                .collect();
            if collect {
                let mut lines = vec![(Text, 99)];
                dispatch_tooltip(&TestHost, &mods, &ItemStack, &mut lines);
                assert_eq!(lines.iter().map(|line| line.1).collect::<Vec<_>>(), [99, 2]);
                assert_eq!(calls.lock().unwrap().len(), 1);
                dispatch_tooltip(&TestHost, &mods, &ItemStack, &mut lines);
                assert_eq!(
                    lines.iter().map(|line| line.1).collect::<Vec<_>>(),
                    [99, 2, 3]
                );
            } else {
                let mut replacement = None;
                dispatch_zoom(&TestHost, &mods, &mut replacement);
                assert_eq!(replacement, None);
                assert_eq!(calls.lock().unwrap().len(), 1);
                dispatch_zoom(&TestHost, &mods, &mut replacement);
                assert_eq!(replacement, Some(NativeZoom(3)));
            }
        }
    }

    #[test]
    fn block_entity_submit_dispatches_in_order_with_shared_draw() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods: Vec<Box<dyn Hooks>> = vec![
            Box::new(Mod {
                id: 1,
                calls: calls.clone(),
            }),
            Box::new(Mod { id: 2, calls }),
        ];
        let mut draw = BlockEntityDraw(Vec::new());
        dispatch_block_entity_submit(
            &TestHost,
            &mods,
            [1, 2, 3],
            "example:machine",
            BlockStateId,
            &ModBlockEntity,
            &mut draw,
        );
        assert_eq!(draw.0, [1, 2]);
    }

    #[test]
    fn slot_rules_mutate_and_item_tick_can_cancel() {
        let mods: Vec<Box<dyn Hooks>> = vec![Box::new(Mod {
            id: 1,
            calls: Arc::new(Mutex::new(Vec::new())),
        })];
        let (mut may_place, mut max_stack) = (true, 64);
        dispatch_menu_slot_rules(
            &mods,
            "test:menu",
            0,
            &ItemStack,
            &mut may_place,
            &mut max_stack,
        );
        assert_eq!((may_place, max_stack), (false, 1));
        let (mut duration, mut canceled) = (10, false);
        dispatch_living_entity_use_item_tick(
            &mut TestHost,
            &mods,
            &ItemStack,
            None,
            Hand,
            &mut duration,
            &mut canceled,
        );
        assert_eq!((duration, canceled), (9, true));
    }

    #[test]
    fn item_finish_can_clear_vanilla_result() {
        let mods: Vec<Box<dyn Hooks>> = vec![Box::new(Mod {
            id: 1,
            calls: Arc::new(Mutex::new(Vec::new())),
        })];
        let mut result = Some(ItemStack);
        dispatch_living_entity_use_item_finish(
            &mut TestHost,
            &mods,
            &ItemStack,
            None,
            Hand,
            0,
            &mut result,
        );
        assert!(result.is_none());
    }

    #[test]
    fn fragment_cancel_stops_later_mods() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods: Vec<Box<dyn Hooks>> = vec![
            Box::new(Mod {
                id: 1,
                calls: calls.clone(),
            }),
            Box::new(Mod {
                id: 2,
                calls: calls.clone(),
            }),
        ];
        let mut canceled = false;
        dispatch_mouse_button_pre(&mut TestHost, &mods, 0, true, 0, &mut canceled);
        assert!(canceled);
        assert_eq!(*calls.lock().unwrap(), [1]);
        // Each call is entered under the hook's name; the cancelled one's is the last.
        let entered = ENTERED.with(|entered| entered.take());
        assert_eq!(entered, [(0, "mouse_button_pre")]);
    }

    #[test]
    fn placement_replacement_can_refuse_and_stops_later_mods() {
        struct Level;
        impl LevelHost for Level {}
        struct Refuse(Arc<Mutex<Vec<u32>>>, u32);
        impl Hooks for Refuse {
            fn state_for_placement(
                &self,
                _: &dyn LevelHost,
                _: &mut StateForPlacementEvent<'_>,
            ) -> Option<Option<BlockStateId>> {
                self.0.lock().unwrap().push(self.1);
                Some(None)
            }
        }
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods: Vec<Box<dyn Hooks>> = vec![
            Box::new(Refuse(calls.clone(), 1)),
            Box::new(Refuse(calls.clone(), 2)),
        ];
        let mut answer = None;
        dispatch_state_for_placement(
            &Level,
            &mods,
            LevelDimension,
            BlockStateId,
            &PlacementContext,
            FluidState,
            &mut answer,
        );
        assert_eq!(answer, Some(None));
        assert_eq!(*calls.lock().unwrap(), vec![1]);
        dispatch_state_for_placement(
            &Level,
            &mods,
            LevelDimension,
            BlockStateId,
            &PlacementContext,
            FluidState,
            &mut answer,
        );
        assert_eq!(*calls.lock().unwrap(), vec![1]);
    }

    #[test]
    fn item_model_property_dispatches_borrowed_stack_and_preserves_unhandled_answers() {
        struct Property;
        impl Hooks for Property {
            fn item_model_property(&self, _: &dyn Host, event: &mut ItemModelPropertyEvent<'_>) {
                if event.property == "example:charged" {
                    event.value = event.stack.count > 1;
                }
            }
        }
        let mods: Vec<Box<dyn Hooks>> = vec![Box::new(Property)];
        let host = TestHost;
        let mut value = false;
        dispatch_item_model_property(
            &host,
            &mods,
            "example:charged",
            &StackView { count: 2 },
            &mut value,
        );
        assert!(value);
        dispatch_item_model_property(
            &host,
            &mods,
            "other:property",
            &StackView { count: 1 },
            &mut value,
        );
        assert!(value);
        dispatch_item_model_property(
            &host,
            &mods,
            "example:charged",
            &StackView { count: 1 },
            &mut value,
        );
        assert!(!value);
        dispatch_item_model_property(
            &host,
            &[],
            "example:charged",
            &StackView { count: 2 },
            &mut value,
        );
        assert!(!value);
    }

    #[test]
    fn dispatch_contracts() {
        struct Weapon;
        impl Hooks for Weapon {
            fn living_knockback_v2(&self, event: &mut LivingKnockbackV2Event<'_>) -> Flow {
                assert_eq!(
                    (event.entity, event.kind, event.attacker),
                    (12, "minecraft:armor_stand", 7)
                );
                assert!(event.attacker_main.is_none() && event.attacker_off.is_none());
                assert_eq!(event.strength, 0.4);
                event.strength *= 2.0;
                event.x = -event.x;
                Flow::Cancel
            }
        }
        let weapon_mods: Vec<Box<dyn Hooks>> = vec![Box::new(Weapon)];
        let (mut strength, mut x, mut z, mut canceled) = (0.4, 1.0, 0.0, false);
        dispatch_living_knockback_v2(
            &weapon_mods,
            12,
            "minecraft:armor_stand",
            7,
            None,
            None,
            &mut strength,
            &mut x,
            &mut z,
            &mut canceled,
        );
        assert_eq!((strength, x, z), (0.8, -1.0, 0.0));
        assert!(canceled);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mods: Vec<Box<dyn Hooks>> = (1..=2)
            .map(|id| {
                Box::new(Mod {
                    id,
                    calls: calls.clone(),
                }) as Box<dyn Hooks>
            })
            .collect();
        let mut table = serde_json::json!([]);
        dispatch_loot_table_load(&mods, "table", &mut table);
        assert_eq!(table, serde_json::json!([1, 2]));
        let mut components = Map::new();
        dispatch_modify_default_components(&mods, "item", &mut components);
        assert_eq!(components["order"], serde_json::json!([1, 2]));
        let mut host = TestHost;
        dispatch_level_tick_pre(&mut host, &mods, "overworld");
        assert_eq!(*calls.lock().unwrap(), [1, 2]);
        calls.lock().unwrap().clear();
        let mut zoom = None;
        dispatch_zoom(&host, &mods, &mut zoom);
        assert_eq!(zoom, Some(NativeZoom(1)));
        assert_eq!(*calls.lock().unwrap(), [1]);
        calls.lock().unwrap().clear();
        dispatch_zoom(&host, &mods, &mut zoom);
        assert!(calls.lock().unwrap().is_empty());
        let mut lines = Vec::new();
        dispatch_tooltip(&host, &mods, &ItemStack, &mut lines);
        assert_eq!(lines.iter().map(|line| line.1).collect::<Vec<_>>(), [1, 2]);
        let mut apply = false;
        calls.lock().unwrap().clear();
        dispatch_should_apply_effect_tick_this_tick(&host, &mods, "effect", 0, 0, &mut apply);
        assert!(!apply);
        assert_eq!(*calls.lock().unwrap(), [1, 2]);
        let mut amount = 1.0;
        let mut canceled = false;
        dispatch_living_incoming_damage(
            &mods,
            0,
            "kind",
            "damage",
            0,
            0,
            &mut amount,
            &mut canceled,
        );
        // Each mod sees the one before: (1 × 2 + 1) × 2 + 2, kept though mod 2 cancels.
        assert_eq!(amount, 8.0);
        assert!(canceled);
        let chained: Vec<Box<dyn Hooks>> = [1, 3]
            .map(|id| {
                Box::new(Mod {
                    id,
                    calls: calls.clone(),
                }) as Box<dyn Hooks>
            })
            .into();
        let mut amount = 1.0;
        let mut canceled = false;
        dispatch_living_incoming_damage(
            &chained,
            0,
            "kind",
            "damage",
            0,
            0,
            &mut amount,
            &mut canceled,
        );
        assert_eq!(amount, 9.0);
        assert!(!canceled);
        calls.lock().unwrap().clear();
        let mut claimed = false;
        dispatch_use_item_on(&mut host, &mods, &ItemStack, [0; 3], [0; 3], &mut claimed);
        assert!(claimed);
        assert_eq!(*calls.lock().unwrap(), [1]);
        calls.lock().unwrap().clear();
        dispatch_use_item_on(&mut host, &mods, &ItemStack, [0; 3], [0; 3], &mut claimed);
        assert!(calls.lock().unwrap().is_empty());
    }
}
