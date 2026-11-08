use super::Game;
use horizon_client::mod_api;

/// The player's bound attack and use buttons during one client tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MouseButtons {
    /// Whether the attack binding is held, regardless of its physical input.
    pub attack_down: bool,
    /// Whether the use binding is held, regardless of its physical input.
    pub use_down: bool,
    /// Suppress vanilla attack handling for this tick.
    pub cancel_vanilla_attack: bool,
    /// Suppress vanilla use handling for this tick.
    pub cancel_vanilla_use: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct MouseButtonsFrame {
    pub(super) cancel_attack: bool,
    pub(super) cancel_use: bool,
    pub(super) cancel_attack_started: bool,
    pub(super) cancel_use_started: bool,
    pub(super) resume_attack: bool,
    pub(super) resume_use: bool,
}

impl MouseButtonsFrame {
    pub(super) fn attack_down(self, down: bool) -> bool {
        down && !self.cancel_attack
    }

    pub(super) fn use_down(self, down: bool) -> bool {
        down && !self.cancel_use
    }

    pub(super) fn attack_clicks(self, clicks: u32) -> u32 {
        if self.cancel_attack {
            0
        } else if self.resume_attack {
            clicks.max(1)
        } else {
            clicks
        }
    }

    pub(super) fn use_clicks(self, clicks: u32) -> u32 {
        if self.cancel_use {
            0
        } else if self.resume_use {
            clicks.max(1)
        } else {
            clicks
        }
    }
}

#[derive(Default)]
pub(super) struct MouseButtonsState {
    attack_cancelled: bool,
    use_cancelled: bool,
    pub(super) frame: MouseButtonsFrame,
}

pub(super) fn dispatch_mouse_buttons_listened(game: &Game<'_>, listened: &mut bool) {
    mod_api::dispatch_mouse_buttons_listened(game, mod_api::mods(), listened);
}

pub(super) fn dispatch_mouse_buttons(game: &mut Game<'_>, buttons: &mut MouseButtons) {
    mod_api::dispatch_mouse_buttons(game, mod_api::mods(), buttons);
}

impl MouseButtonsState {
    pub(super) fn poll(
        &mut self,
        attack_down: bool,
        use_down: bool,
        listened: bool,
        dispatch: impl FnOnce(&mut MouseButtons),
    ) -> MouseButtonsFrame {
        let (cancel_attack, cancel_use) = if listened {
            let mut buttons = MouseButtons {
                attack_down,
                use_down,
                ..Default::default()
            };
            dispatch(&mut buttons);
            (buttons.cancel_vanilla_attack, buttons.cancel_vanilla_use)
        } else {
            (false, false)
        };
        self.frame = MouseButtonsFrame {
            cancel_attack,
            cancel_use,
            cancel_attack_started: cancel_attack && !self.attack_cancelled,
            cancel_use_started: cancel_use && !self.use_cancelled,
            resume_attack: self.attack_cancelled && !cancel_attack && attack_down,
            resume_use: self.use_cancelled && !cancel_use && use_down,
        };
        self.attack_cancelled = cancel_attack;
        self.use_cancelled = cancel_use;
        self.frame
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

pub(super) fn poll(game: &mut Game<'_>, eligible: bool) -> MouseButtonsFrame {
    if !eligible || !game.native_window() {
        game.mouse_buttons_state.reset();
        return MouseButtonsFrame::default();
    }

    let (attack_down, use_down) = (game.left, game.right);
    let mut listened = false;
    game.native_mouse_buttons_listened(&mut listened);
    let mut state = std::mem::take(&mut game.mouse_buttons_state);
    let frame = state.poll(attack_down, use_down, listened, |buttons| {
        game.native_mouse_buttons(buttons);
    });
    game.mouse_buttons_state = state;
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_listener_preserves_down_states_and_queued_clicks() {
        let mut state = MouseButtonsState::default();
        for (attack, use_item, attack_clicks, use_clicks) in [
            (true, false, 1, 0),
            (true, true, 0, 3),
            (false, true, 2, 0),
            (false, false, 0, 0),
        ] {
            let frame = state.poll(attack, use_item, false, |_| {
                panic!("the gated hook must not run without a listener")
            });
            assert_eq!(frame.attack_down(attack), attack);
            assert_eq!(frame.use_down(use_item), use_item);
            assert_eq!(frame.attack_clicks(attack_clicks), attack_clicks);
            assert_eq!(frame.use_clicks(use_clicks), use_clicks);
            assert_eq!(frame, MouseButtonsFrame::default());
        }
    }

    #[test]
    fn cancellation_filters_each_held_tick_and_resumes_with_one_edge() {
        let mut state = MouseButtonsState::default();
        for (index, clicks) in [1, 0, 3].into_iter().enumerate() {
            let frame = state.poll(true, true, true, |buttons| {
                assert!(buttons.attack_down);
                assert!(buttons.use_down);
                buttons.cancel_vanilla_attack = true;
                buttons.cancel_vanilla_use = true;
            });
            assert!(!frame.attack_down(true));
            assert!(!frame.use_down(true));
            assert_eq!(frame.cancel_attack_started, index == 0);
            assert_eq!(frame.cancel_use_started, index == 0);
            assert_eq!(frame.attack_clicks(clicks), 0);
            assert_eq!(frame.use_clicks(clicks), 0);
        }
        let frame = state.poll(true, true, true, |_| {});
        assert!(frame.attack_down(true));
        assert!(frame.use_down(true));
        assert_eq!(frame.attack_clicks(0), 1);
        assert_eq!(frame.use_clicks(0), 1);
        assert_eq!(frame.attack_clicks(2), 2);
        assert_eq!(frame.use_clicks(2), 2);
        let next = state.poll(true, true, true, |_| {});
        assert_eq!(next.attack_clicks(0), 0);
        assert_eq!(next.use_clicks(0), 0);
    }

    #[test]
    fn the_two_cancellations_are_independent() {
        let mut state = MouseButtonsState::default();
        let frame = state.poll(true, true, true, |buttons| {
            buttons.cancel_vanilla_attack = true;
        });
        assert!(!frame.attack_down(true));
        assert_eq!(frame.attack_clicks(2), 0);
        assert!(frame.use_down(true));
        assert_eq!(frame.use_clicks(2), 2);
        let frame = state.poll(true, true, true, |buttons| {
            buttons.cancel_vanilla_use = true;
        });
        assert!(frame.attack_down(true));
        assert_eq!(frame.attack_clicks(0), 1);
        assert!(!frame.use_down(true));
        assert_eq!(frame.use_clicks(2), 0);
    }

    #[test]
    fn releasing_or_opening_a_screen_does_not_create_a_press() {
        let mut state = MouseButtonsState::default();
        state.poll(true, true, true, |buttons| {
            buttons.cancel_vanilla_attack = true;
            buttons.cancel_vanilla_use = true;
        });
        let released = state.poll(false, false, true, |_| {});
        assert_eq!(released.attack_clicks(0), 0);
        assert_eq!(released.use_clicks(0), 0);
        state.poll(true, true, true, |buttons| {
            buttons.cancel_vanilla_attack = true;
            buttons.cancel_vanilla_use = true;
        });
        state.reset();
        let after_screen = state.poll(true, true, true, |_| {});
        assert_eq!(after_screen.attack_clicks(0), 0);
        assert_eq!(after_screen.use_clicks(0), 0);
    }

    #[test]
    fn removing_the_listener_while_held_resumes_once() {
        let mut state = MouseButtonsState::default();
        state.poll(true, true, true, |buttons| {
            buttons.cancel_vanilla_attack = true;
            buttons.cancel_vanilla_use = true;
        });
        let frame = state.poll(true, true, false, |_| {
            panic!("the removed listener must not run")
        });
        assert_eq!(frame.attack_clicks(0), 1);
        assert_eq!(frame.use_clicks(0), 1);
        let frame = state.poll(true, true, false, |_| unreachable!());
        assert_eq!(frame.attack_clicks(0), 0);
        assert_eq!(frame.use_clicks(0), 0);
    }

    #[derive(Default)]
    struct DispatchTestHost {
        sent: Vec<(String, Vec<u8>)>,
    }

    impl mod_api::Host for DispatchTestHost {
        fn net_send_to_server(&mut self, channel: &str, data: &[u8]) -> Result<(), String> {
            self.sent.push((channel.to_owned(), data.to_vec()));
            Ok(())
        }
    }

    struct DispatchTestMod {
        calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl mod_api::Hooks for DispatchTestMod {
        fn mouse_buttons_listened(
            &self,
            _host: &dyn mod_api::Host,
            event: &mut mod_api::MouseButtonsListenedEvent,
        ) {
            event.listened = true;
        }

        fn mouse_buttons(
            &self,
            host: &mut dyn mod_api::Host,
            event: &mut mod_api::MouseButtonsEvent<'_>,
        ) {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            assert!(event.buttons.attack_down);
            assert!(!event.buttons.use_down);
            event.buttons.cancel_vanilla_attack = true;
            host.net_send_to_server("neon:buttons", &[1]).unwrap();
        }
    }

    #[test]
    fn generated_dispatch_gates_and_applies_mod_cancellation() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let mut host = DispatchTestHost::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let mods: Vec<Box<dyn mod_api::Hooks>> = vec![Box::new(DispatchTestMod {
            calls: Arc::clone(&calls),
        })];
        let mut listened = false;
        mod_api::dispatch_mouse_buttons_listened(&host, &mods, &mut listened);
        assert!(listened);
        let mut state = MouseButtonsState::default();
        let frame = state.poll(true, false, listened, |buttons| {
            mod_api::dispatch_mouse_buttons(&mut host, &mods, buttons);
        });
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(host.sent, vec![("neon:buttons".to_owned(), vec![1])]);
        assert_eq!(frame.attack_clicks(1), 0);
        assert!(!frame.attack_down(true));
        assert!(!frame.cancel_use);
    }

    #[test]
    fn generated_dispatch_without_listeners_preserves_input() {
        struct UninterestedMod;
        impl mod_api::Hooks for UninterestedMod {}

        let host = DispatchTestHost::default();
        let mods: Vec<Box<dyn mod_api::Hooks>> = vec![Box::new(UninterestedMod)];
        let mut listened = false;
        mod_api::dispatch_mouse_buttons_listened(&host, &mods, &mut listened);
        assert!(!listened);
        let mut state = MouseButtonsState::default();
        let frame = state.poll(true, true, listened, |_| unreachable!());
        assert!(frame.attack_down(true));
        assert!(frame.use_down(true));
        assert_eq!(frame.attack_clicks(1), 1);
        assert_eq!(frame.use_clicks(1), 1);
    }

    #[derive(Debug, Default, PartialEq, Eq)]
    struct TestInputWorld {
        attacks: u32,
        uses: u32,
        picks: u32,
        held_attacks: u32,
        use_delay: u32,
        destroying: bool,
        using_item: bool,
        aborts: u32,
        releases: u32,
    }

    impl horizon_client::interaction_input::InteractionWorld for TestInputWorld {
        fn pick_hit_result(&mut self) {}

        fn release_hit_result(&mut self) {}

        fn using_item(&self) -> bool {
            self.using_item
        }

        fn release_using(&mut self) {
            self.using_item = false;
            self.releases += 1;
        }

        fn start_attack(&mut self) -> bool {
            self.attacks += 1;
            false
        }

        fn start_use(&mut self) {
            self.uses += 1;
            self.use_delay = 4;
        }

        fn pick(&mut self) {
            self.picks += 1;
        }

        fn use_delay(&self) -> u32 {
            self.use_delay
        }

        fn continue_attack(&mut self, down: bool) {
            if down {
                self.held_attacks += 1;
            } else if self.destroying {
                self.destroying = false;
                self.aborts += 1;
            }
        }
    }

    fn tick_input(
        input: &mut horizon_client::interaction_input::InteractionInput,
        world: &mut TestInputWorld,
        frame: MouseButtonsFrame,
        attack_down: bool,
        use_down: bool,
    ) {
        use horizon_client::interaction_input::Button;

        input.filter_clicks(Button::Attack, |clicks| frame.attack_clicks(clicks));
        input.filter_clicks(Button::Use, |clicks| frame.use_clicks(clicks));
        input.tick(
            world,
            frame.attack_down(attack_down),
            frame.use_down(use_down),
        );
    }

    #[test]
    fn input_without_a_listener_matches_unmodified_handling() {
        use horizon_client::interaction_input::{Button, InteractionInput};

        let mut normal = InteractionInput::default();
        let mut filtered = InteractionInput::default();
        for button in [Button::Attack, Button::Use, Button::Pick] {
            normal.press(button);
            filtered.press(button);
        }
        let mut normal_world = TestInputWorld::default();
        let mut filtered_world = TestInputWorld::default();
        let mut state = MouseButtonsState::default();
        let frame = state.poll(true, true, false, |_| unreachable!());
        normal.tick(&mut normal_world, true, true);
        tick_input(&mut filtered, &mut filtered_world, frame, true, true);
        assert_eq!(filtered_world, normal_world);
    }

    #[test]
    fn input_cancellation_suppresses_actions_and_preserves_pick() {
        use horizon_client::interaction_input::{Button, InteractionInput};

        let mut input = InteractionInput::default();
        let mut world = TestInputWorld::default();
        let mut state = MouseButtonsState::default();
        for _ in 0..3 {
            input.press(Button::Attack);
            input.press(Button::Use);
            input.press(Button::Pick);
            let frame = state.poll(true, true, true, |buttons| {
                buttons.cancel_vanilla_attack = true;
                buttons.cancel_vanilla_use = true;
            });
            tick_input(&mut input, &mut world, frame, true, true);
        }
        assert_eq!(world.attacks, 0);
        assert_eq!(world.uses, 0);
        assert_eq!(world.held_attacks, 0);
        assert_eq!(world.picks, 3);

        let resumed = state.poll(true, true, true, |_| {});
        tick_input(&mut input, &mut world, resumed, true, true);
        assert_eq!(world.attacks, 1);
        assert_eq!(world.uses, 1);
        assert_eq!(world.held_attacks, 1);
        let held = state.poll(true, true, true, |_| {});
        tick_input(&mut input, &mut world, held, true, true);
        assert_eq!(world.attacks, 1);
        assert_eq!(world.uses, 1);
        assert_eq!(world.held_attacks, 2);
    }

    #[test]
    fn filtered_attack_cancellation_releases_mock_mining_once() {
        use horizon_client::interaction_input::InteractionInput;

        let mut input = InteractionInput::default();
        let mut world = TestInputWorld {
            destroying: true,
            ..Default::default()
        };
        let mut state = MouseButtonsState::default();
        let held = state.poll(true, false, true, |_| {});
        tick_input(&mut input, &mut world, held, true, false);
        assert!(world.destroying);
        assert_eq!(world.aborts, 0);

        for _ in 0..2 {
            let cancelled = state.poll(true, false, true, |buttons| {
                buttons.cancel_vanilla_attack = true;
            });
            tick_input(&mut input, &mut world, cancelled, true, false);
        }
        assert!(!world.destroying);
        assert_eq!(world.aborts, 1);
        assert_eq!(world.releases, 0);
    }

    #[test]
    fn filtered_use_cancellation_releases_mock_item_once() {
        use horizon_client::interaction_input::InteractionInput;

        let mut input = InteractionInput::default();
        let mut world = TestInputWorld {
            using_item: true,
            ..Default::default()
        };
        let mut state = MouseButtonsState::default();
        let held = state.poll(false, true, true, |_| {});
        tick_input(&mut input, &mut world, held, false, true);
        assert!(world.using_item);
        assert_eq!(world.releases, 0);

        for _ in 0..2 {
            let cancelled = state.poll(false, true, true, |buttons| {
                buttons.cancel_vanilla_use = true;
            });
            tick_input(&mut input, &mut world, cancelled, false, true);
        }
        assert!(!world.using_item);
        assert_eq!(world.releases, 1);
        assert_eq!(world.aborts, 0);
    }
}
