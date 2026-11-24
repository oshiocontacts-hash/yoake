//! Input method (IME) text: committed composition goes to the focused text
//! field through the same sinks the keyboard and `horizon-ctl type` use.
//! The window only asks the OS for an input method while such a field has
//! focus, so movement keys keep arriving as plain key events.
use super::{Game, Screen, books::BookScreen};
use horizon_client::{
    hud::HudBatch,
    preedit::{self, PreeditEvent},
};

impl Game<'_> {
    /// Whether a text box has focus right now (the fields `ime_commit` can fill).
    pub(super) fn text_field_focused(&mut self) -> bool {
        // REM-12: a full-access module took the input method (`ime_set_allowed(true)`)
        if super::yoake2_f_state::ime_claimed() {
            return true;
        }
        if self.native_mod_screen().is_some()
            && let Some(allowed) =
                mod_text_field_decision(self.mod_ime_allowed, self.title_mod_screen_open())
        {
            return allowed;
        }
        if self.dialog_open() {
            return self.dialog_has_text_field();
        }
        if let Some(title) = self.title.as_mut() {
            // A caret field, an append-only one (`focused_field`), or a game
            // rule's box (`has_caret_field` asks the rules screen itself, and
            // `title_edit` types into it).
            return title.has_caret_field() || title.focused_field().is_some();
        }
        if self.sign_edit.is_some() || matches!(self.book, Some(BookScreen::Edit { .. })) {
            return true;
        }
        match self.screen {
            Screen::Chat | Screen::CommandBlock | Screen::Social => true,
            Screen::DebugOptions => self
                .debug_options
                .as_ref()
                .is_some_and(|s| s.search_focused()),
            // The language list's search, the settings search and the pack
            // screen's search (OptionsUi::typing).
            Screen::Options => self.ui.options.typing(),
            Screen::WorldOptions => self.world_options_typing(),
            Screen::Creative => self.creative_screen.tab().search,
            Screen::BlockEditor => self.block_editor.as_ref().is_some_and(|s| s.text_focused()),
            Screen::Inventory => {
                self.mod_browser_typing() || self.recipe_search_focused() || self.station_typing()
            }
            _ => false,
        }
    }

    /// Allow the input method only while a text box has focus.
    pub(super) fn sync_ime(&mut self) {
        let want = self.text_field_focused();
        if want != self.ime_allowed
            && let Some(window) = &self.window
        {
            window.set_ime_allowed(want);
            self.ime_allowed = want;
            // The field lost the focus (or a new one took it): the composition
            // and the candidate window's place belong to the old one.
            self.ime_area = None;
            if !want {
                preedit::set_preedit(None);
            }
        }
    }

    /// `WindowEvent::Ime(Preedit)`: `KeyboardHandler.textEditing`. The text
    /// still being composed is shown under the focused field's caret.
    pub(super) fn ime_preedit(&mut self, text: &str, cursor: Option<(usize, usize)>) {
        if self.yoake2_ime_preedit_hook(text, cursor) {
            preedit::set_preedit(None);
            return;
        }
        if self.native_mod_screen().is_some() {
            self.native_ime_preedit(text, mod_ime_cursor(cursor));
        }
        preedit::set_preedit(
            self.text_field_focused()
                .then(|| PreeditEvent::from_winit(text, cursor))
                .flatten(),
        );
    }

    pub(super) fn ime_disabled(&mut self) {
        self.yoake2_ime_preedit_hook("", None);
        preedit::set_preedit(None);
        if self.native_mod_screen().is_some() {
            self.native_ime_preedit("", None);
        }
    }

    /// @API:ime@ Enable the OS input method for the calling mod's open screen.
    pub(super) fn native_ime_set_allowed(
        &mut self,
        mod_id: &str,
        allowed: bool,
    ) -> Result<(), String> {
        require_mod_ime_owner(self.native_mod_screen(), mod_id)?;
        self.mod_ime_allowed = Some(allowed);
        if !allowed {
            self.mod_ime_area = None;
            preedit::set_preedit(None);
        }
        Ok(())
    }

    /// @API:ime@ Set the candidate window's GUI-scaled rectangle on the calling mod's screen.
    pub(super) fn native_ime_set_cursor_area(
        &mut self,
        mod_id: &str,
        area: [f32; 4],
    ) -> Result<(), String> {
        require_mod_ime_owner(self.native_mod_screen(), mod_id)?;
        validate_mod_ime_area(area)?;
        self.mod_ime_area = Some(area);
        Ok(())
    }

    /// With the frame's GUI built: draw the composition over it and move the
    /// input method's candidate window to the caret
    /// (`TextInputManager.setTextInputArea`, in window pixels).
    pub(super) fn flush_ime(&mut self, hud: &mut HudBatch) {
        // REM-12: a full-access module owns the input method: no engine overlay, the module's candidate area
        if super::yoake2_f_state::ime_claimed() {
            let vertex_start = hud.vertices.len();
            preedit::draw_overlay(hud, &self.gui.atlas);
            hud.vertices.truncate(vertex_start);
            let _ = preedit::take_text_input_area();
            match super::yoake2_f_state::ime_area() {
                Some(area) => self.update_ime_area(physical_mod_ime_area(area, hud.scale)),
                None => self.ime_area = None,
            }
            return;
        }
        let mode = flush_mode(
            self.native_mod_screen().is_some(),
            self.title_mod_screen_open(),
            self.mod_ime_used(),
            self.mod_ime_area.is_some(),
        );
        if mode != FlushMode::Vanilla {
            let vertex_start = hud.vertices.len();
            preedit::draw_overlay(hud, &self.gui.atlas);
            hud.vertices.truncate(vertex_start);
            let _ = preedit::take_text_input_area();
            match (mode, self.mod_ime_area) {
                (FlushMode::ModArea, Some(area)) => {
                    self.update_ime_area(physical_mod_ime_area(area, hud.scale))
                }
                _ => self.ime_area = None,
            }
            return;
        }
        preedit::draw_overlay(hud, &self.gui.atlas);
        let Some([x0, y0, x1, y1]) = preedit::take_text_input_area() else {
            return;
        };
        let scale = hud.scale;
        let x = (x0 * scale).round() as i32;
        let y = (y0 * scale).round() as i32;
        let width = (((x1 - x0) * scale).round() as i32).max(1);
        let height = (((y1 - y0) * scale).round() as i32).max(1);
        self.update_ime_area([x, y, width, height]);
    }

    fn update_ime_area(&mut self, [x, y, width, height]: [i32; 4]) {
        let area = [x, y, width, height];
        if self.ime_area == Some(area) {
            return;
        }
        if let Some(window) = &self.window
            && self.ime_allowed
        {
            window.set_ime_cursor_area(
                winit::dpi::PhysicalPosition::new(x, y),
                winit::dpi::PhysicalSize::new(width as u32, height as u32),
            );
            self.ime_area = Some(area);
        }
    }

    /// Text the input method committed (`WindowEvent::Ime(Commit)`).
    pub(super) fn ime_commit(&mut self, text: &str) {
        if self.yoake2_ime_commit_hook(text) {
            preedit::set_preedit(None);
            return;
        }
        if self.native_mod_screen().is_some() {
            self.native_ime_commit(text);
        }
        let mod_screen = commit_stops_at_mod_screen(
            self.native_mod_screen().is_some(),
            self.title_mod_screen_open(),
            self.mod_ime_used(),
        );
        preedit::set_preedit(None);
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        // ScreenEvent.CharacterTyped.Pre: the input method's committed text
        // reaches a text box by characters, as a key's own text does, so the
        // open screen hears of each of them first and a cancelled one never
        // reaches the box.
        let Some(text) = self.screen_text_typed(&text) else {
            return;
        };
        if mod_screen {
            return;
        }
        if text.is_empty() || !self.text_field_focused() {
            return;
        }
        if self.dialog_type(&text) {
            return;
        }
        if self.title.is_some() {
            self.title_edit(Some(&text), false);
        } else if self.book.is_some() {
            self.book_input(None, Some(&text));
        } else if self.sign_edit.is_some() {
            self.sign_input(None, Some(&text));
        } else {
            match self.screen {
                Screen::Chat => self.chat_type(&text),
                Screen::CommandBlock => self.command_block_type(&text),
                Screen::Social => self.social_type(&text),
                Screen::DebugOptions => self.debug_options_type(&text),
                Screen::Options => self.ui.options.type_text(&self.gui.atlas, &text),
                Screen::WorldOptions => self.world_options_text(Some(&text)),
                Screen::Creative => self.creative_search_edit(Some(&text)),
                Screen::BlockEditor => self.block_editor_type(&text),
                Screen::Inventory if self.mod_browser_typing() => {
                    self.mod_browser_text(Some(&text), false)
                }
                Screen::Inventory if self.recipe_search_focused() => {
                    self.recipe_text(Some(&text), false)
                }
                Screen::Inventory => self.station_text(Some(&text), false),
                _ => {}
            }
        }
    }
}

impl Game<'_> {
    /// Whether the open mod screen's mod called an IME service. Until it does,
    /// the screen behaves exactly as it did before the services existed.
    fn mod_ime_used(&self) -> bool {
        mod_ime_used(self.mod_ime_allowed, self.mod_ime_area.is_some())
    }
}

fn mod_ime_used(allowed: Option<bool>, has_area: bool) -> bool {
    allowed.is_some() || has_area
}

/// `Some(answer)` for `text_field_focused` on a mod screen, `None` to fall
/// through to the ordinary checks. Not claimed by the mod: the old rule (a
/// title mod screen takes IME, an in-game one falls through).
fn mod_text_field_decision(allowed: Option<bool>, title_mod: bool) -> Option<bool> {
    match allowed {
        Some(allowed) => Some(allowed),
        None => title_mod.then_some(true),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlushMode {
    /// The engine's own overlay and candidate area.
    Vanilla,
    /// Hide the engine overlay, no candidate area (the old title mod screen rule).
    ModHidden,
    /// Hide the engine overlay, use the mod's rectangle.
    ModArea,
}

fn flush_mode(mod_screen: bool, title_mod: bool, used: bool, has_area: bool) -> FlushMode {
    if mod_screen && used {
        if has_area {
            FlushMode::ModArea
        } else {
            FlushMode::ModHidden
        }
    } else if title_mod {
        FlushMode::ModHidden
    } else {
        FlushMode::Vanilla
    }
}

/// Whether `ime_commit` ends after the character hooks (the mod screen owns
/// the text), as it did for a title mod screen before.
fn commit_stops_at_mod_screen(mod_screen: bool, title_mod: bool, used: bool) -> bool {
    title_mod || (mod_screen && used)
}

fn mod_ime_cursor(cursor: Option<(usize, usize)>) -> Option<(u32, u32)> {
    cursor.and_then(|(start, end)| {
        Some((u32::try_from(start).ok()?, u32::try_from(end).ok()?))
    })
}

fn validate_mod_ime_area(area: [f32; 4]) -> Result<(), String> {
    if area.iter().all(|value| value.is_finite()) && area[2] >= 0.0 && area[3] >= 0.0 {
        Ok(())
    } else {
        Err("IME cursor area needs finite coordinates and nonnegative size".to_owned())
    }
}

fn physical_mod_ime_area([x, y, width, height]: [f32; 4], scale: f32) -> [i32; 4] {
    [
        (x * scale).round() as i32,
        (y * scale).round() as i32,
        ((width * scale).round() as i32).max(1),
        ((height * scale).round() as i32).max(1),
    ]
}

fn require_mod_ime_owner(owner: Option<&str>, mod_id: &str) -> Result<(), String> {
    if owner == Some(mod_id) {
        Ok(())
    } else {
        Err("IME services require the calling mod's open screen".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unused_mod_screen_keeps_old_text_field_rule() {
        // Old: title mod screen -> true; in-game mod screen -> ordinary checks.
        assert_eq!(mod_text_field_decision(None, true), Some(true));
        assert_eq!(mod_text_field_decision(None, false), None);
    }

    #[test]
    fn used_mod_screen_follows_the_mod() {
        assert_eq!(mod_text_field_decision(Some(false), true), Some(false));
        assert_eq!(mod_text_field_decision(Some(true), false), Some(true));
        assert!(!mod_ime_used(None, false));
        assert!(mod_ime_used(Some(false), false));
        assert!(mod_ime_used(None, true));
    }

    #[test]
    fn flush_mode_matches_old_rules_when_unused() {
        assert_eq!(flush_mode(true, true, false, false), FlushMode::ModHidden);
        assert_eq!(flush_mode(true, false, false, false), FlushMode::Vanilla);
        assert_eq!(flush_mode(false, false, false, false), FlushMode::Vanilla);
        assert_eq!(flush_mode(true, false, true, false), FlushMode::ModHidden);
        assert_eq!(flush_mode(true, false, true, true), FlushMode::ModArea);
        assert_eq!(flush_mode(true, true, true, true), FlushMode::ModArea);
    }

    #[test]
    fn commit_stop_matches_old_rules_when_unused() {
        assert!(commit_stops_at_mod_screen(true, true, false));
        assert!(!commit_stops_at_mod_screen(true, false, false));
        assert!(!commit_stops_at_mod_screen(false, false, false));
        assert!(commit_stops_at_mod_screen(true, false, true));
    }

    #[test]
    fn mod_screen_owner_is_required() {
        assert!(require_mod_ime_owner(Some("demo"), "demo").is_ok());
        assert!(require_mod_ime_owner(Some("other"), "demo").is_err());
        assert!(require_mod_ime_owner(None, "demo").is_err());
    }

    #[test]
    fn cursor_uses_wit_u32_byte_offsets() {
        assert_eq!(mod_ime_cursor(Some((2, 5))), Some((2, 5)));
        assert_eq!(mod_ime_cursor(None), None);
        #[cfg(target_pointer_width = "64")]
        assert_eq!(mod_ime_cursor(Some((0, u32::MAX as usize + 1))), None);
    }

    #[test]
    fn cursor_area_validation_rejects_bad_numbers() {
        assert!(validate_mod_ime_area([10.0, 20.0, 0.0, 0.0]).is_ok());
        for area in [
            [f32::NAN, 0.0, 1.0, 1.0],
            [0.0, f32::INFINITY, 1.0, 1.0],
            [0.0, 0.0, -1.0, 1.0],
            [0.0, 0.0, 1.0, -0.1],
        ] {
            assert!(validate_mod_ime_area(area).is_err());
        }
    }

    #[test]
    fn cursor_area_scales_like_vanilla_ime() {
        assert_eq!(
            physical_mod_ime_area([10.2, -4.6, 20.4, 0.0], 2.0),
            [20, -9, 41, 1]
        );
    }
}
