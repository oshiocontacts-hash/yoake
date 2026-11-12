use super::{NativeMod, dll_mods, entry, mod_api, tick};
use horizon_mod_sdk::{Flow, Host, ScreenView, YoakeMod};
use std::{cell::Cell, ffi::OsStr, path::PathBuf, sync::Arc, time::Instant};

fn example_library(name: &str, variable: &str) -> PathBuf {
    if let Some(path) = std::env::var_os(variable) {
        let path = PathBuf::from(path);
        assert!(path.is_absolute(), "{variable} must name an absolute DLL path");
        return path.canonicalize().unwrap_or_else(|error| panic!("{variable}: {error}"));
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let target = if target.is_absolute() { target } else { root.join(target) };
    let mut directories = vec![target.clone(), target.join(super::abi::TARGET)];
    if let Some(triple) = std::env::var_os("CARGO_BUILD_TARGET") {
        directories.insert(0, target.join(triple));
    }
    for directory in directories {
        for profile in ["debug", "release"] {
            let candidate = directory.join(profile).join(format!("{name}.dll"));
            if candidate.is_file() {
                return candidate.canonicalize().expect("canonical example DLL path");
            }
        }
    }
    panic!(
        "Missing {name}.dll: build `cargo build -p yoake-dll-hello -p yoake-dll-net-echo` \
         from horizon with the same checkout/target, or set {variable} to its absolute path"
    );
}

fn load(path: &std::path::Path) -> (mod_api::Mods, &'static [NativeMod]) {
    dll_mods::from_library(Arc::from([]), &[], path, Some(OsStr::new("1")), b"yoake_mod_entry\0")
        .unwrap_or_else(|error| panic!("loading {}: {error}", path.display()))
}

#[test]
fn real_hello_dll_requires_unsigned_opt_in_and_receives_tick_and_hud() {
    let path = example_library("dll_hello", "YOAKE_TEST_DLL_HELLO");
    for flag in [None, Some(OsStr::new("0"))] {
        let refused = dll_mods::from_library(Arc::from([]), &[], &path, flag, b"yoake_mod_entry\0");
        let error = refused.err().expect("unsigned DLL must be refused without exactly 1");
        assert!(error.contains("Unsigned"), "{error}");
    }
    let (mods, identities) = load(&path);
    assert_eq!(identities.last().unwrap().id, "dll-hello");
    let mut host = mod_api::FakeHost { client: true, ..Default::default() };
    tick(&mods, &mut host);
    let mut batch = mod_api::HudBatch::default();
    mods[0].hud(&mut host, &mut mod_api::HudEvent { batch: &mut batch });
    assert_eq!(batch.text, ["dll-hello #1 [F6]"]);
    assert_eq!(batch.rects, [[6.0, 6.0, 260.0, 18.0]]);
    tick(&mods, &mut host);
    let mut next = mod_api::HudBatch::default();
    mods[0].hud(&mut host, &mut mod_api::HudEvent { batch: &mut next });
    assert_eq!(next.text, ["dll-hello #2 [F6]"]);
}

thread_local! {
    static LATER_VISITS: Cell<usize> = const { Cell::new(0) };
}

#[derive(Default)]
struct Later;

impl YoakeMod for Later {
    const ID: &'static str = "real-dll-later";
    const VERSION: &'static str = "1";

    fn screen_key_pressed_pre(
        &mut self,
        _host: &mut Host<'_>,
        _screen: ScreenView<'_>,
        _key: &str,
        _repeat: bool,
        _modifiers: u32,
    ) -> Flow {
        LATER_VISITS.with(|visits| visits.set(visits.get() + 1));
        Flow::Continue
    }
}

#[test]
fn real_net_echo_dll_listens_copies_frames_draws_and_cancels_later_mods() {
    let path = example_library("dll_net_echo", "YOAKE_TEST_DLL_NET_ECHO");
    let (mods, identities) = load(&path);
    let (mods, _) = unsafe { dll_mods::in_process(mods, identities, &[entry::<Later>]).unwrap() };
    let mut host = mod_api::FakeHost { client: true, ..Default::default() };
    let mut listened = false;
    for mod_ in &*mods {
        let mut event = mod_api::NetFramesListenedEvent { listened }; mod_.net_frames_listened(&mut event); listened = event.listened;
    }
    assert!(listened);
    let mut body = [0, 128, 255];
    {
        let frame = mod_api::NetFrameReceived {
            state: mod_api::NetState::Play,
            id: 77,
            body: &body,
            at: Instant::now(),
            session: 5,
        };
        for mod_ in &*mods {
            mod_.net_frame_received(&mut mod_api::NetFrameReceivedEvent { frame: &frame });
        }
    }
    body.fill(0);
    tick(&mods, &mut host);
    assert_eq!(host.feedback, ["dll-net-echo tick 1 frames 1"]);
    let mut batch = mod_api::HudBatch::default();
    for mod_ in &*mods {
        mod_.hud(&mut host, &mut mod_api::HudEvent { batch: &mut batch });
    }
    assert_eq!(batch.text, ["dll-net-echo 1:77 [0, 128, 255]"]);
    assert_eq!(batch.rects, [[6.0, 28.0, 300.0, 18.0]]);
    LATER_VISITS.with(|visits| visits.set(0));
    for (id, expected) in [("dll-net-echo", true), ("another-screen", false)] {
        let screen = mod_api::NativeScreenView {
            id,
            rect: [0.0, 0.0, 640.0, 480.0],
            mouse: [0.0, 0.0],
            partial_tick: 0.0,
        };
        let mut event = mod_api::ScreenKeyPressedPreEvent {
            screen: &screen,
            key: "key.keyboard.escape",
            repeat: false,
            modifiers: 0,
        };
        let cancelled = mods.iter().any(|mod_| {
            mod_.screen_key_pressed_pre(&host, &mut event) == mod_api::Flow::Cancel
        });
        assert_eq!(cancelled, expected);
        LATER_VISITS.with(|visits| assert_eq!(visits.get(), usize::from(!expected)));
    }
}

#[test]
fn real_dll_wrong_abi_header_is_refused() {
    let path = example_library("dll_net_echo", "YOAKE_TEST_DLL_NET_ECHO");
    let refused = dll_mods::from_library(
        Arc::from([]),
        &[],
        &path,
        Some(OsStr::new("1")),
        b"yoake_mod_entry_wrong_abi\0",
    );
    let error = refused.err().expect("wrong ABI DLL must be refused");
    assert!(error.contains("descriptor ABI version, schema or size"), "{error}");
}
