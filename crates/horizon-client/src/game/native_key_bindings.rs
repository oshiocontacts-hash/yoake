use super::{Game, Screen};

pub(super) fn accepts_press(
    playing: bool,
    screen_owner: Option<&str>,
    focused: bool,
    id: &str,
) -> bool {
    !focused && (playing || owned_binding(id, screen_owner))
}

pub(super) fn owned_binding(id: &str, owner: Option<&str>) -> bool {
    horizon_client::keybinds::mod_mapping_name(id).is_some()
        && owner.is_some_and(|owner| id.split_once(':').is_some_and(|parts| parts.0 == owner))
}

impl Game<'_> {
    pub(super) fn native_mod_binding_event(&mut self, key: &str, pressed: bool) {
        let focused = self.text_field_focused();
        let playing = self.screen == Screen::Playing && self.title.is_none() && !self.dialog_open();
        let owner = (self.screen == Screen::Mod)
            .then_some(self.native_mod_screen.as_deref())
            .flatten();
        for id in self.settings.keys.mod_key_event_when(key, pressed, |id| {
            accepts_press(playing, owner, focused, id)
        }) {
            self.native_key_binding_pressed(&id);
        }
    }

    pub fn native_register_key_binding(
        &mut self,
        id: &str,
        default_key: &str,
        category: &str,
        label: &str,
    ) -> Result<(), String> {
        if !owned_binding(
            id,
            horizon_client::crash::native_mod_call().and_then(|call| call.id),
        ) {
            return Err("key binding id must belong to the calling mod".to_owned());
        }
        self.settings
            .keys
            .register_mod_binding(id, default_key, category, label)
    }

    pub fn native_key_binding_state(&self, id: &str) -> bool {
        owned_binding(
            id,
            horizon_client::crash::native_mod_call().and_then(|call| call.id),
        ) && self.settings.keys.mod_key_state(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_registration_is_namespaced() {
        assert!(owned_binding("example:action", Some("example")));
        assert!(!owned_binding("example:action", Some("other")));
        assert!(!owned_binding("example:action", None));
        assert!(!owned_binding("not an id", Some("example")));
    }

    #[test]
    fn presses_are_private_to_gameplay_or_the_owning_screen_without_text_focus() {
        assert!(accepts_press(true, None, false, "example:action"));
        assert!(!accepts_press(false, None, false, "example:action"));
        assert!(!accepts_press(true, None, true, "example:action"));
        assert!(accepts_press(
            false,
            Some("example"),
            false,
            "example:action"
        ));
        assert!(!accepts_press(
            false,
            Some("other"),
            false,
            "example:action"
        ));
        assert!(!accepts_press(
            false,
            Some("example"),
            true,
            "example:action"
        ));
    }
}
