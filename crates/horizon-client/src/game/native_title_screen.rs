use super::{Game, Screen, glfw_button, glfw_modifiers};
use horizon_client::{hud::HudBatch, preedit};
use std::time::Instant;
use winit::{
    event::{ElementState, Ime, MouseScrollDelta, WindowEvent},
    keyboard::{KeyCode, PhysicalKey},
};

impl Game<'_> {
    pub(super) fn mod_screen_open_anywhere(&mut self, mod_id: &str) -> bool {
        if uses_world_mod_screen_opener(self.screen, self.title.is_some()) {
            return self.native_mod_screen_open(mod_id);
        }
        if self.screen == Screen::Mod {
            return self.native_mod_screen() == Some(mod_id);
        }
        let blocked = title_mode_blocks(
            self.screen,
            self.title
                .as_ref()
                .is_some_and(|title| !title.allows_mod_screen()),
        )
            || (self.title.is_none() && self.player.is_none())
            || self.renderer.is_none()
            || self.screen_over_playing()
            || self.remote_loading()
            || self.lan_acting.is_some()
            || self.pending_save.is_some()
            || self.pack_reload.is_some()
            || self.pending_pack_reload
            || self.ui.options.resource_error.is_some()
            || self
                .ui
                .options
                .pack_screen
                .as_ref()
                .is_some_and(|screen| screen.confirming());
        let from_settings = self.screen == Screen::Options;
        if !open_menu_mod_screen(
            &mut self.screen,
            &mut self.native_title_mod_screen_return,
            &mut self.native_mod_screen,
            mod_id,
            self.title.is_some(),
            blocked,
        ) {
            return false;
        }
        if from_settings {
            self.native_settings_menu = Some(self.pad.menu.suspend_for_mod_screen());
        }
        self.keys.clear();
        self.left = false;
        self.right = false;
        self.mining = None;
        self.dragging = false;
        self.drag_button = None;
        self.pad.menu.osk = None;
        self.ime_area = None;
        preedit::set_preedit(None);
        self.regrab_on_refocus = false;
        self.capture(false);
        true
    }

    pub(super) fn close_native_mod_screen(&mut self) {
        self.native_mod_screen = None;
        let title_return = self.native_title_mod_screen_return.take();
        if let Some((screen, capture)) =
            close_destination(self.screen, self.title.is_some(), title_return)
        {
            self.screen = screen;
            if screen == Screen::Options
                && let Some(menu) = self.native_settings_menu.take()
            {
                self.pad.menu.resume_from_mod_screen(menu);
            }
            self.capture(capture);
        } else {
            self.native_settings_menu = None;
        }
    }

    pub(super) fn title_mod_screen_open(&self) -> bool {
        self.title.is_some() && self.screen == Screen::Mod
    }

    pub(super) fn handle_title_mod_screen_event(&mut self, event: &WindowEvent) -> bool {
        if !blocks_title_input(self.title_mod_screen_open(), event) {
            return false;
        }
        self.last_input = Instant::now();
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                if let PhysicalKey::Code(key) = event.physical_key {
                    let canceled = if pressed {
                        self.screen_key_pressed(key, event.repeat)
                    } else {
                        self.screen_key_released(key)
                    };
                    if closes_on_key(key, event.state, canceled) {
                        self.close_mod_screen();
                    }
                }
                if self.title_mod_screen_open()
                    && pressed
                    && self.types_text()
                    && let Some(text) = &event.text
                {
                    let _ = self.screen_text_typed(text);
                }
            }
            WindowEvent::Ime(Ime::Commit(text)) => self.ime_commit(text),
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => self.ime_preedit(text, *cursor),
            WindowEvent::Ime(Ime::Disabled) => preedit::set_preedit(None),
            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = *state == ElementState::Pressed;
                if !self.pad_synthetic_input() {
                    self.track_drag_button(*button, pressed);
                    if let Some(screen) = self.screen_view_now() {
                        let mut canceled = false;
                        let modifiers = glfw_modifiers(self.modifiers);
                        if pressed {
                            self.native_screen_mouse_button_pressed_pre(
                                &screen,
                                glfw_button(*button),
                                modifiers,
                                &mut canceled,
                            );
                        } else {
                            self.native_screen_mouse_button_released_pre(
                                &screen,
                                glfw_button(*button),
                                modifiers,
                                &mut canceled,
                            );
                        }
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let previous = self.cursor;
                if let Some(renderer) = &self.renderer {
                    let batch = HudBatch::new([renderer.config.width, renderer.config.height]);
                    self.cursor = (
                        position.x as f32 / batch.scale,
                        position.y as f32 / batch.scale,
                    );
                }
                self.screen_mouse_dragged(previous);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if !self.pad_synthetic_input()
                    && let Some(screen) = self.screen_view_now()
                {
                    let mut canceled = false;
                    self.native_screen_mouse_scrolled_pre(
                        &screen,
                        scroll_delta(
                            delta,
                            self.settings.discrete_scroll,
                            self.settings.mouse_wheel_sensitivity,
                        ),
                        glfw_modifiers(self.modifiers),
                        &mut canceled,
                    );
                }
            }
            _ => {}
        }
        true
    }
}

fn title_mode_blocks(screen: Screen, title_restricted: bool) -> bool {
    screen != Screen::Options && title_restricted
}

fn uses_world_mod_screen_opener(screen: Screen, title_open: bool) -> bool {
    !title_open && !matches!(screen, Screen::Options | Screen::Paused)
}

fn menu_can_open(screen: Screen, title_open: bool, blocked: bool) -> bool {
    !blocked
        && (matches!(screen, Screen::Playing | Screen::Options)
            || (!title_open && screen == Screen::Paused))
}

fn open_menu_mod_screen(
    screen: &mut Screen,
    return_screen: &mut Option<Screen>,
    owner: &mut Option<String>,
    mod_id: &str,
    title_open: bool,
    blocked: bool,
) -> bool {
    if !menu_can_open(*screen, title_open, blocked) {
        return false;
    }
    *return_screen = Some(*screen);
    *owner = Some(mod_id.to_owned());
    *screen = Screen::Mod;
    true
}

pub(super) fn mod_screen_pauses(return_screen: Option<Screen>) -> bool {
    matches!(return_screen, Some(Screen::Options | Screen::Paused))
}

pub(super) fn settings_suspended(screen: Screen, return_screen: Option<Screen>) -> bool {
    screen == Screen::Mod && return_screen == Some(Screen::Options)
}

fn close_destination(
    screen: Screen,
    title_open: bool,
    title_return: Option<Screen>,
) -> Option<(Screen, bool)> {
    (screen == Screen::Mod).then(|| {
        if let Some(previous) = title_return {
            (previous, false)
        } else if title_open {
            (Screen::Playing, false)
        } else {
            (Screen::Playing, true)
        }
    })
}

fn closes_on_key(key: KeyCode, state: ElementState, canceled: bool) -> bool {
    key == KeyCode::Escape && state == ElementState::Pressed && !canceled
}

fn blocks_title_input(modal: bool, event: &WindowEvent) -> bool {
    modal
        && matches!(
            event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::Ime(_)
                | WindowEvent::MouseInput { .. }
                | WindowEvent::CursorMoved { .. }
                | WindowEvent::CursorEntered { .. }
                | WindowEvent::CursorLeft { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::Touch(_)
                | WindowEvent::DroppedFile(_)
                | WindowEvent::HoveredFile(_)
                | WindowEvent::HoveredFileCancelled
        )
}

fn scroll_delta(delta: &MouseScrollDelta, discrete: bool, sensitivity: f32) -> [f64; 2] {
    let [horizontal, vertical] = match delta {
        MouseScrollDelta::LineDelta(horizontal, vertical) => {
            [f64::from(*horizontal), f64::from(*vertical)]
        }
        MouseScrollDelta::PixelDelta(position) => [position.x / 30.0, position.y / 30.0],
    };
    [horizontal, vertical].map(|value| {
        let value = if discrete {
            horizon_client::mouse::java_signum(value)
        } else {
            value
        };
        value * f64::from(sensitivity)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::native_deferred::DeferredRequests;
    use winit::{dpi::PhysicalSize, event::DeviceId};

    #[test]
    fn no_mod_preserves_title_dispatch() {
        let events = [
            WindowEvent::Ime(Ime::Commit("unchanged".into())),
            WindowEvent::MouseWheel {
                device_id: DeviceId::dummy(),
                delta: MouseScrollDelta::LineDelta(0.0, 1.0),
                phase: winit::event::TouchPhase::Moved,
            },
            WindowEvent::DroppedFile("pack.zip".into()),
            WindowEvent::HoveredFileCancelled,
            WindowEvent::Resized(PhysicalSize::new(800, 600)),
            WindowEvent::Focused(true),
            WindowEvent::CloseRequested,
            WindowEvent::RedrawRequested,
        ];
        for event in events {
            // CHECK: no modal means every event remains on the existing title path.
            assert!(!blocks_title_input(false, &event));
        }
    }

    #[test]
    fn mod_consumes_title_input_but_not_window_lifecycle() {
        assert!(blocks_title_input(
            true,
            &WindowEvent::Ime(Ime::Commit("mod text".into()))
        ));
        assert!(blocks_title_input(
            true,
            &WindowEvent::DroppedFile("pack.zip".into())
        ));
        assert!(!blocks_title_input(true, &WindowEvent::Focused(false)));
        assert!(!blocks_title_input(true, &WindowEvent::RedrawRequested));
        assert!(!blocks_title_input(true, &WindowEvent::CloseRequested));
    }

    #[test]
    fn title_gate_rejects_loading_and_other_screens() {
        assert!(!title_mode_blocks(Screen::Options, true));
        assert!(title_mode_blocks(Screen::Playing, true));
        assert!(!title_mode_blocks(Screen::Playing, false));
        assert!(uses_world_mod_screen_opener(Screen::Playing, false));
        assert!(uses_world_mod_screen_opener(Screen::Inventory, false));
        assert!(!uses_world_mod_screen_opener(Screen::Options, false));
        assert!(!uses_world_mod_screen_opener(Screen::Paused, false));
        assert!(!uses_world_mod_screen_opener(Screen::Playing, true));
        assert!(menu_can_open(Screen::Playing, true, false));
        assert!(menu_can_open(Screen::Options, false, false));
        assert!(menu_can_open(Screen::Paused, false, false));
        assert!(!menu_can_open(Screen::Paused, true, false));
        assert!(!menu_can_open(Screen::Paused, false, true));
        assert!(!menu_can_open(Screen::Playing, true, true));
        assert!(!menu_can_open(Screen::Options, false, true));
        assert!(!menu_can_open(Screen::Inventory, false, false));
        assert!(!menu_can_open(Screen::Mod, false, false));
    }

    #[test]
    fn deferred_pause_menu_request_opens_and_keeps_singleplayer_paused() {
        let requests = DeferredRequests::default();
        let mut screen = Screen::Paused;
        let mut return_screen = None;
        let mut owner = None;

        assert!(requests.request_mod_screen(Some(0)));
        assert_eq!(requests.take_mod_screen(), Some(0));
        assert!(!uses_world_mod_screen_opener(screen, false));
        assert!(open_menu_mod_screen(
            &mut screen,
            &mut return_screen,
            &mut owner,
            "nestgate",
            false,
            false,
        ));
        assert!(screen == Screen::Mod);
        assert_eq!(owner.as_deref(), Some("nestgate"));
        assert!(mod_screen_pauses(return_screen));
        assert!(mod_screen_pauses(Some(Screen::Options)));
        assert!(!mod_screen_pauses(Some(Screen::Playing)));
        assert!(!mod_screen_pauses(None));
        assert!(matches!(
            close_destination(screen, false, return_screen),
            Some((Screen::Paused, false))
        ));
        assert_eq!(requests.take_mod_screen(), None);
    }

    #[test]
    fn suspended_settings_preserve_calibration() {
        assert!(settings_suspended(Screen::Mod, Some(Screen::Options)));
        assert!(!settings_suspended(Screen::Options, None));
        assert!(!settings_suspended(Screen::Mod, Some(Screen::Playing)));
        assert!(!settings_suspended(Screen::Mod, None));
    }

    #[test]
    fn escape_restores_title_without_mouse_capture() {
        assert!(closes_on_key(KeyCode::Escape, ElementState::Pressed, false));
        assert!(!closes_on_key(
            KeyCode::Escape,
            ElementState::Released,
            false
        ));
        assert!(!closes_on_key(KeyCode::Escape, ElementState::Pressed, true));
        assert!(matches!(
            close_destination(Screen::Mod, true, Some(Screen::Options)),
            Some((Screen::Options, false))
        ));
        assert!(matches!(
            close_destination(Screen::Mod, false, Some(Screen::Options)),
            Some((Screen::Options, false))
        ));
        assert!(matches!(
            close_destination(Screen::Mod, true, None),
            Some((Screen::Playing, false))
        ));
        assert!(matches!(
            close_destination(Screen::Mod, false, None),
            Some((Screen::Playing, true))
        ));
        assert!(close_destination(Screen::Options, true, None).is_none());
    }

    #[test]
    fn modal_scroll_uses_existing_gui_units() {
        assert_eq!(
            scroll_delta(&MouseScrollDelta::LineDelta(0.5, -2.0), false, 2.0),
            [1.0, -4.0]
        );
        assert_eq!(
            scroll_delta(&MouseScrollDelta::LineDelta(0.5, -2.0), true, 2.0),
            [2.0, -2.0]
        );
    }
}
