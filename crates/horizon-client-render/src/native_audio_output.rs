use crate::Result;

#[derive(Default)]
pub(crate) struct OutputChoice {
    configured: Option<String>,
}

pub(crate) fn devices(enabled: bool, enumerate: impl FnOnce() -> Vec<String>) -> Vec<String> {
    if !enabled {
        return Vec::new();
    }
    enumerate()
}

pub(crate) fn reopen_output<Stream>(
    output: &mut Option<Stream>,
    requested: &mut String,
    target: &str,
    open: impl FnOnce(&str) -> Result<Stream>,
) -> Result<()> {
    let stream = open(target)?;
    *output = Some(stream);
    *requested = target.to_owned();
    Ok(())
}

impl OutputChoice {
    pub(crate) fn configured<'name>(&'name self, requested: &'name str) -> &'name str {
        self.configured.as_deref().unwrap_or(requested)
    }

    pub(crate) fn clear(&mut self) {
        self.configured = None;
    }

    pub(crate) fn select<Stream>(
        &mut self,
        enabled: bool,
        output: &mut Option<Stream>,
        requested: &mut String,
        device: Option<&str>,
        open: impl FnOnce(&str) -> Result<Stream>,
    ) -> Result<()> {
        if !enabled {
            return Err("audio output selection requires an installed client mod".into());
        }
        if device == Some("") {
            return Err("audio output name is empty; use None for the system default".into());
        }
        let configured = self.configured.is_none().then(|| requested.clone());
        reopen_output(output, requested, device.unwrap_or(""), open)?;
        if let Some(configured) = configured {
            self.configured = Some(configured);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeBackend {
        opened: Vec<String>,
    }

    impl FakeBackend {
        fn open(&mut self, target: &str) -> Result<usize> {
            self.opened.push(target.to_owned());
            if target == "unavailable" {
                return Err("device cannot open".into());
            }
            Ok(self.opened.len())
        }
    }

    #[test]
    fn no_mod_does_not_enumerate_or_reopen_output() {
        assert!(devices(false, || panic!("inactive enumeration")).is_empty());
        let mut choice = OutputChoice::default();
        let mut output = Some(7);
        let mut requested = "configured".to_owned();
        assert!(
            choice
                .select(false, &mut output, &mut requested, Some("other"), |_| {
                    panic!("inactive reopen")
                })
                .is_err()
        );
        assert_eq!(output, Some(7));
        assert_eq!(requested, "configured");
        assert!(choice.configured.is_none());
    }

    #[test]
    fn mod_enumerates_selects_named_and_returns_to_system_default() {
        assert_eq!(
            devices(true, || vec!["speakers".into(), "headphones".into()]),
            ["speakers", "headphones"]
        );
        let mut backend = FakeBackend::default();
        let mut choice = OutputChoice::default();
        let mut output = None;
        let mut requested = "configured".to_owned();
        choice
            .select(
                true,
                &mut output,
                &mut requested,
                Some("headphones"),
                |target| backend.open(target),
            )
            .unwrap();
        assert_eq!(output, Some(1));
        assert_eq!(requested, "headphones");
        assert_eq!(choice.configured(&requested), "configured");
        choice
            .select(true, &mut output, &mut requested, None, |target| {
                backend.open(target)
            })
            .unwrap();
        assert_eq!(output, Some(2));
        assert_eq!(requested, "");
        assert_eq!(choice.configured(&requested), "configured");
        assert_eq!(backend.opened, ["headphones", ""]);
    }

    #[test]
    fn failed_selection_preserves_the_stream_and_preferences() {
        let mut backend = FakeBackend::default();
        let mut choice = OutputChoice::default();
        let mut output = Some(7);
        let mut requested = "speakers".to_owned();
        assert!(
            choice
                .select(
                    true,
                    &mut output,
                    &mut requested,
                    Some("unavailable"),
                    |target| backend.open(target),
                )
                .is_err()
        );
        assert_eq!(output, Some(7));
        assert_eq!(requested, "speakers");
        assert!(choice.configured.is_none());
        assert!(
            choice
                .select(true, &mut output, &mut requested, Some(""), |_| {
                    panic!("empty device must not open")
                })
                .is_err()
        );
    }

    #[test]
    fn device_loss_uses_the_same_replacement_without_losing_the_override() {
        let mut backend = FakeBackend::default();
        let mut choice = OutputChoice::default();
        let mut output = None;
        let mut requested = "configured".to_owned();
        choice
            .select(
                true,
                &mut output,
                &mut requested,
                Some("headphones"),
                |target| backend.open(target),
            )
            .unwrap();
        let target = requested.clone();
        reopen_output(&mut output, &mut requested, &target, |target| {
            backend.open(target)
        })
        .unwrap();
        assert_eq!(output, Some(2));
        assert_eq!(backend.opened, ["headphones", "headphones"]);
        assert_eq!(choice.configured(&requested), "configured");
        choice.clear();
        assert_eq!(choice.configured(&requested), "headphones");
    }
}
