use horizon_mod_sdk::{Flow, Host, HudBatch, NetFrameReceived, ScreenView, YoakeMod, abi, yoake_mod};

#[derive(Default)]
struct NetEcho {
    ticks: u64,
    frames: u64,
    last_packet: i32,
    last_body: Vec<u8>,
}

impl YoakeMod for NetEcho {
    const ID: &'static str = "dll-net-echo";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    fn client_tick(&mut self, host: &mut Host<'_>) {
        self.ticks = self.ticks.saturating_add(1);
        let _ = host.feedback(&format!("dll-net-echo tick {} frames {}", self.ticks, self.frames));
    }

    fn net_frames_listened(&mut self, listened: &mut bool) {
        *listened = true;
    }

    fn net_frame_received(&mut self, frame: NetFrameReceived<'_>) {
        self.frames = self.frames.saturating_add(1);
        self.last_packet = frame.packet_id;
        self.last_body = frame.body.to_vec();
    }

    fn hud(&mut self, host: &mut Host<'_>, mut batch: HudBatch<'_>) {
        let line = format!("dll-net-echo {}:{} {:?}", self.frames, self.last_packet, self.last_body);
        let _ = host.fill(&mut batch, [6.0, 28.0, 300.0, 18.0], 0x78000000);
        let _ = host.text(&mut batch, &line, [10.0, 33.0], 0xffffff, true);
    }

    fn screen_key_pressed_pre(&mut self, _host: &mut Host<'_>, screen: ScreenView<'_>, key: &str, _repeat: bool, _modifiers: u32) -> Flow {
        if screen.id == Self::ID && key == "key.keyboard.escape" { Flow::Cancel } else { Flow::Continue }
    }
}

yoake_mod!(NetEcho);

static WRONG_ABI: abi::TableHeader = abi::TableHeader {
    abi_version: abi::ABI_VERSION + 1,
    size: std::mem::size_of::<abi::ModDescriptor>() as u32,
    schema_hash: abi::SCHEMA_HASH,
};

#[unsafe(no_mangle)]
pub extern "C" fn yoake_mod_entry_wrong_abi(_version: u32, _host: *const abi::HostTableV1) -> *const abi::ModDescriptor {
    std::ptr::from_ref(&WRONG_ABI).cast()
}
