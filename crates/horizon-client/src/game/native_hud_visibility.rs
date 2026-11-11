#[derive(Default)]
pub(super) struct HudVisibility {
    hidden: bool,
}

impl HudVisibility {
    pub(super) fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
    }

    pub(super) fn hidden(&self) -> bool {
        self.hidden
    }

    pub(super) fn visible(&self, vanilla_hidden: bool) -> bool {
        !vanilla_hidden && !self.hidden
    }
}

#[cfg(test)]
mod tests {
    use super::HudVisibility;

    #[test]
    fn default_visibility_preserves_vanilla_f1() {
        let visibility = HudVisibility::default();
        assert!(visibility.visible(false));
        assert!(!visibility.visible(true));
    }

    #[test]
    fn mod_can_hide_and_restore_without_overriding_f1() {
        let mut visibility = HudVisibility::default();
        visibility.set_hidden(true);
        assert!(visibility.hidden());
        assert!(!visibility.visible(false));
        visibility.set_hidden(false);
        assert!(visibility.visible(false));
        assert!(!visibility.visible(true));
    }
}
