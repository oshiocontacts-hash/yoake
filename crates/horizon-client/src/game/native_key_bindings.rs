use super::Game;

pub(super) fn owned_binding(id: &str, owner: Option<&str>) -> bool {
    horizon_client::keybinds::mod_mapping_name(id).is_some()
        && owner.is_some_and(|owner| id.split_once(':').is_some_and(|parts| parts.0 == owner))
}

impl Game<'_> {
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
        self.settings.keys.mod_key_state(id)
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
}
