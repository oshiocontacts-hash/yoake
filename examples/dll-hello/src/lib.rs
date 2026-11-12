use horizon_mod_sdk::{Host, HudBatch, YoakeMod, yoake_mod};

struct Hello {
    ticks: u64,
    visible: bool,
    key_down: bool,
    restored: bool,
    in_world: bool,
}

impl Default for Hello {
    fn default() -> Self {
        Self {
            ticks: 0,
            visible: true,
            key_down: false,
            restored: false,
            in_world: false,
        }
    }
}

impl YoakeMod for Hello {
    const ID: &'static str = "dll-hello";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn client_tick(&mut self, host: &mut Host<'_>) {
        self.ticks = self.ticks.saturating_add(1);
        if !self.restored {
            if let Ok(Some(value)) = host.setting("visible") {
                self.visible = value != "false";
            }
            self.restored = true;
        }
        self.in_world = matches!(host.client_world_key(), Ok(Some(_)));
    }

    fn key_input(&mut self, host: &mut Host<'_>, key: &str, pressed: bool) {
        if key != "key.keyboard.f6" {
            return;
        }
        if pressed && !self.key_down {
            self.visible = !self.visible;
            let _ = host.set_setting("visible", if self.visible { "true" } else { "false" });
            let _ = host.feedback(if self.visible { "[F6] +" } else { "[F6] -" });
            if self.in_world {
                let _ = host.mod_net_send("dll-hello:echo", b"hello from the startup DLL");
            }
        }
        self.key_down = pressed;
    }

    fn hud(&mut self, host: &mut Host<'_>, batch: HudBatch<'_>) {
        if !self.visible {
            return;
        }
        let line = format!("dll-hello #{} [F6]", self.ticks);
        let _ = host.hud_rect(&batch, [6.0, 6.0, 260.0, 18.0], 0x78000000);
        let _ = host.hud_text(&batch, &line, [10.0, 11.0], 0xffffff, true);
    }

    fn mod_net_receive(&mut self, host: &mut Host<'_>, channel: &str, data: &[u8]) {
        if channel == "dll-hello:echo" {
            let _ = host.mod_net_send(channel, data);
        }
    }
}

yoake_mod!(Hello);
