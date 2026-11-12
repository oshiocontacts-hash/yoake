use super::{abi, dll_mods, entry, mod_api, tick};
use horizon_mod_sdk::{Flow, GuiNineSlice, Host, HudBatch, NetFrameReceived, NetState, ScreenView, SettingsButtons, YoakeMod};
use std::{cell::Cell, path::PathBuf, sync::Arc, time::{Duration, Instant}};

#[derive(Default)]
struct Services;

impl YoakeMod for Services {
    const ID: &'static str = "phase2-services";
    const VERSION: &'static str = "1";

    fn client_tick(&mut self, host: &mut Host<'_>) {
        assert_eq!(host.playing(), Ok(true));
        assert_eq!(host.text_width("A界"), Ok(Some(12.0)));
        assert_eq!(host.instance_dir(), Ok(PathBuf::from("C:/Yoake/fixture")));
        let account = host.account().unwrap().unwrap();
        assert_eq!((account.name.as_str(), account.uuid, account.online), ("fixture", 123, true));
        let player = host.local_player_snapshot().unwrap().unwrap();
        assert_eq!(player.entity_id, 42);
        assert_eq!(player.uuid, 0x112233445566778899aabbccddeeff00);
        assert_eq!(player.pos, [1.5, 64.0, -2.5]);
        assert_eq!([player.yaw, player.pitch, player.head_yaw, player.body_yaw], [10.0, 20.0, 30.0, 40.0]);
        assert_eq!(player.flags, (1 << 10) - 1);
        assert_eq!((player.pose, player.swing_hand, player.swing_ticks), (3, abi::HAND_OFF, 7));
        assert_eq!((player.using_hand, player.use_ticks), (abi::HAND_MAIN, 9));
        assert!(player.equipment.iter().all(|slot| slot.as_slice() == [0]));
        assert_eq!((player.vehicle, player.skin_parts), (Some(17), 127));
        assert_eq!((player.dimension.as_str(), player.game_mode), ("minecraft:overworld", 1));
        assert_eq!((player.health, player.max_health, player.food_level), (18.5, Some(20.0), 19));
        let mut frames = Vec::new();
        let report = host.snapshot_client_world(|state, packet, bytes| frames.push((state, packet, bytes.to_vec()))).unwrap();
        assert_eq!(frames, [(NetState::Configuration, 4, vec![0, 1, 255]), (NetState::Play, 7, vec![2, 3])]);
        assert_eq!(report.written, 2);
        assert_eq!(report.missing, ["optional fixture state"]);
        assert_eq!(host.mod_screen(), Ok(None));
        assert_eq!(host.mod_screen_open(Self::ID), Ok(true));
        assert_eq!(host.mod_screen(), Ok(Some(Self::ID.into())));
        host.mod_screen_close(Self::ID).unwrap();
        assert_eq!(host.mod_screen(), Ok(None));
        assert_eq!(host.mod_screen_open_anywhere(Self::ID), Ok(true));
        host.mod_screen_close(Self::ID).unwrap();
        host.feedback("services passed").unwrap();
    }

    fn hud(&mut self, host: &mut Host<'_>, mut batch: HudBatch<'_>) {
        assert_eq!(batch.len(), Ok(0));
        assert_eq!(batch.is_empty(), Ok(true));
        host.fill(&mut batch, [1.0, 2.0, 3.0, 4.0], 0xff112233).unwrap();
        let start = batch.len().unwrap();
        host.text(&mut batch, "phase 2", [3.0, 4.0], 0xffffff, true).unwrap();
        batch.scale_since(start, 3.0, 4.0, 2.0, 2.0).unwrap();
        batch.clip_since(start, 1.0, 2.0, 30.0, 40.0).unwrap();
        batch.tint_since(start, 0.5).unwrap();
        assert_eq!(batch.scale_since(usize::MAX, 0.0, 0.0, 1.0, 1.0), Err(abi::Status::INVALID));
        let texture = host.gui_texture_create([16, 16], abi::TEXTURE_RGBA8, abi::FILTER_NEAREST).unwrap();
        assert_eq!(host.gui_texture(&mut batch, &texture, [1.0, 2.0, 3.0, 4.0], [0.0, 0.0, 1.0, 1.0], 0xffffffff, Some(GuiNineSlice { source_size: [16, 16], border: [2; 4] })), Ok(true));
        host.gui_texture_release(texture).unwrap();
        assert_eq!(batch.len(), Ok(18));
    }
}

#[test]
fn replay_services_and_hud_facade_round_trip_without_touching_earlier_vertices() {
    let (mods, _) = unsafe { dll_mods::in_process(Arc::from([]), &[], &[entry::<Services>]).unwrap() };
    let mut host = mod_api::FakeHost { client: true, ..Default::default() };
    tick(&mods, &mut host);
    let mut batch = mod_api::HudBatch::default();
    batch.vertices.push(mod_api::HudVertex { color: 0xffabcdef });
    mods[0].hud(&mut host, &mut mod_api::HudEvent { batch: &mut batch });
    assert_eq!(host.feedback, ["services passed"]);
    assert_eq!(batch.text, ["phase 2"]);
    assert_eq!(batch.rects, [[1.0, 2.0, 3.0, 4.0]]);
    assert_eq!(batch.scales, [(7, [3.0, 4.0, 2.0, 2.0])]);
    assert_eq!(batch.clips, [(7, [1.0, 2.0, 30.0, 40.0])]);
    assert_eq!(batch.vertices[0].color, 0xffabcdef);
    assert_eq!(batch.vertices[1].color, 0xff112233);
    assert_eq!(batch.vertices[7].color, 0x80ffffff);
    assert_eq!(batch.textures, 1);
    assert_eq!(batch.vertices.len(), 19);
}

#[derive(Default)]
struct HookProbe { frame: bool, local_tick: bool, packet: Vec<u8>, ended: bool }

fn screen(view: ScreenView<'_>) {
    assert_eq!(view.id, "phase2-hooks");
    assert_eq!(view.rect, [1.0, 2.0, 640.0, 480.0]);
    assert_eq!(view.mouse, [50.0, 60.0]);
    assert_eq!(view.partial_tick, 0.25);
}

fn read_only(host: &mut Host<'_>) {
    assert_eq!(host.playing(), Ok(true));
    assert_eq!(host.feedback("must not write"), Err(abi::Status::UNSUPPORTED));
    assert_eq!(host.mod_screen_open("phase2-hooks"), Err(abi::Status::UNSUPPORTED));
}

impl YoakeMod for HookProbe {
    const ID: &'static str = "phase2-hooks";
    const VERSION: &'static str = "1";

    fn frame(&mut self, _host: &mut Host<'_>, elapsed: Duration) { assert_eq!(elapsed, Duration::from_micros(16667)); self.frame = true; }
    fn local_player_tick_end(&mut self, _host: &mut Host<'_>) { self.local_tick = true; }
    fn net_frames_listened(&mut self, listened: &mut bool) { *listened = true; }
    fn net_frame_received(&mut self, frame: NetFrameReceived<'_>) {
        assert_eq!((frame.state, frame.packet_id, frame.session), (NetState::Play, 77, 123));
        self.packet = frame.body.to_vec();
    }
    fn net_session_ended(&mut self, session: u64, reason: &str) { assert_eq!((session, reason), (123, "done")); self.ended = true; }
    fn gui_layer_pre(&mut self, host: &mut Host<'_>, layer: &str, mut batch: HudBatch<'_>) -> Flow {
        assert_eq!(layer, "overlay");
        host.text(&mut batch, "pre", [0.0, 0.0], 0xffffff, false).unwrap();
        Flow::Cancel
    }
    fn gui_layer_post(&mut self, host: &mut Host<'_>, layer: &str, mut batch: HudBatch<'_>) {
        assert_eq!(layer, "overlay");
        host.text(&mut batch, "post", [0.0, 0.0], 0xffffff, false).unwrap();
    }
    fn screen_render_post(&mut self, host: &mut Host<'_>, view: ScreenView<'_>, mut batch: HudBatch<'_>) {
        screen(view); read_only(host);
        host.text(&mut batch, "screen", [0.0, 0.0], 0xffffff, false).unwrap();
    }
    fn screen_key_pressed_pre(&mut self, host: &mut Host<'_>, view: ScreenView<'_>, key: &str, repeat: bool, modifiers: u32) -> Flow {
        screen(view); read_only(host);
        assert_eq!((key, repeat, modifiers), ("key.keyboard.escape", true, 3));
        Flow::Cancel
    }
    fn screen_char_typed_pre(&mut self, host: &mut Host<'_>, view: ScreenView<'_>, character: char, modifiers: u32) -> Flow {
        screen(view); read_only(host); assert_eq!((character, modifiers), ('界', 3)); Flow::Cancel
    }
    fn screen_mouse_button_pressed_pre(&mut self, host: &mut Host<'_>, view: ScreenView<'_>, button: i32, modifiers: u32) -> Flow {
        screen(view); read_only(host); assert_eq!((button, modifiers), (1, 3)); Flow::Cancel
    }
    fn screen_mouse_button_released_pre(&mut self, host: &mut Host<'_>, view: ScreenView<'_>, button: i32, modifiers: u32) -> Flow {
        screen(view); read_only(host); assert_eq!((button, modifiers), (1, 3)); Flow::Cancel
    }
    fn screen_mouse_scrolled_pre(&mut self, host: &mut Host<'_>, view: ScreenView<'_>, scroll: [f64; 2], modifiers: u32) -> Flow {
        screen(view); read_only(host); assert_eq!((scroll, modifiers), ([1.25, -2.5], 3)); Flow::Cancel
    }
    fn settings(&mut self, _host: &mut Host<'_>, mod_id: &str, buttons: &mut SettingsButtons<'_>) { assert_eq!(mod_id, Self::ID); buttons.push("first").unwrap(); }
    fn settings_click(&mut self, host: &mut Host<'_>, mod_id: &str, button: usize) {
        assert_eq!((mod_id, button), (Self::ID, 0));
        assert!(self.frame && self.local_tick && self.ended);
        assert_eq!(self.packet, [0, 128, 255]);
        host.feedback("hooks passed").unwrap();
    }
}

thread_local! { static LATE_CANCELS: Cell<usize> = const { Cell::new(0) }; }

#[derive(Default)]
struct Later;

macro_rules! late_cancel {
    ($name:ident $(, $argument:ident: $kind:ty)*) => {
        fn $name(&mut self, _host: &mut Host<'_>, _screen: ScreenView<'_>, $($argument: $kind),*) -> Flow {
            $(let _ = $argument;)*
            LATE_CANCELS.set(LATE_CANCELS.get() + 1);
            Flow::Continue
        }
    };
}

impl YoakeMod for Later {
    const ID: &'static str = "phase2-later";
    const VERSION: &'static str = "1";
    fn net_frames_listened(&mut self, listened: &mut bool) { *listened = false; }
    fn gui_layer_pre(&mut self, _host: &mut Host<'_>, _layer: &str, _batch: HudBatch<'_>) -> Flow { LATE_CANCELS.set(LATE_CANCELS.get() + 1); Flow::Continue }
    late_cancel!(screen_key_pressed_pre, key: &str, repeat: bool, modifiers: u32);
    late_cancel!(screen_char_typed_pre, character: char, modifiers: u32);
    late_cancel!(screen_mouse_button_pressed_pre, button: i32, modifiers: u32);
    late_cancel!(screen_mouse_button_released_pre, button: i32, modifiers: u32);
    late_cancel!(screen_mouse_scrolled_pre, scroll: [f64; 2], modifiers: u32);
    fn settings(&mut self, _host: &mut Host<'_>, _mod_id: &str, buttons: &mut SettingsButtons<'_>) { buttons.push("second").unwrap(); }
}

macro_rules! cancel {
    ($mods:expr, $hook:ident, $host:expr, $event:expr) => {{
        let mut event = $event;
        $mods.iter().any(|mod_| mod_.$hook($host, &mut event) == mod_api::Flow::Cancel)
    }};
}

#[test]
fn added_hooks_preserve_borrowed_frames_cancel_skip_and_settings_collection() {
    LATE_CANCELS.set(0);
    let (mods, _) = unsafe { dll_mods::in_process(Arc::from([]), &[], &[entry::<HookProbe>, entry::<Later>]).unwrap() };
    let mut host = mod_api::FakeHost { client: true, ..Default::default() };
    let mut batch = mod_api::HudBatch::default();
    let view = mod_api::NativeScreenView { id: HookProbe::ID, rect: [1.0, 2.0, 640.0, 480.0], mouse: [50.0, 60.0], partial_tick: 0.25 };
    let mut listened = false;
    for mod_ in &*mods { let mut event = mod_api::NetFramesListenedEvent { listened }; mod_.net_frames_listened(&mut event); listened = event.listened; }
    assert!(listened);
    let mut body = [0, 128, 255];
    {
        let frame = mod_api::NetFrameReceived { state: mod_api::NetState::Play, id: 77, body: &body, at: Instant::now(), session: 123 };
        for mod_ in &*mods { mod_.net_frame_received(&mut mod_api::NetFrameReceivedEvent { frame: &frame }); }
    }
    body.fill(0);
    for mod_ in &*mods {
        mod_.frame(&mut host, &mut mod_api::FrameEvent { elapsed: Duration::from_micros(16667) });
        mod_.local_player_tick_end(&mut host, &mut mod_api::LocalPlayerTickEndEvent);
        mod_.net_session_ended(&mut mod_api::NetSessionEndedEvent { session: 123, reason: "done" });
        mod_.gui_layer_post(&mut host, &mut mod_api::GuiLayerPostEvent { layer: "overlay", batch: &mut batch });
        mod_.screen_render_post(&host, &mut mod_api::ScreenRenderPostEvent { screen: &view, batch: &mut batch });
    }
    assert!(cancel!(mods, gui_layer_pre, &mut host, mod_api::GuiLayerPreEvent { layer: "overlay", batch: &mut batch }));
    assert!(cancel!(mods, screen_key_pressed_pre, &host, mod_api::ScreenKeyPressedPreEvent { screen: &view, key: "key.keyboard.escape", repeat: true, modifiers: 3 }));
    assert!(cancel!(mods, screen_char_typed_pre, &host, mod_api::ScreenCharTypedPreEvent { screen: &view, c: '界', modifiers: 3 }));
    assert!(cancel!(mods, screen_mouse_button_pressed_pre, &host, mod_api::ScreenMouseButtonPressedPreEvent { screen: &view, button: 1, modifiers: 3 }));
    assert!(cancel!(mods, screen_mouse_button_released_pre, &host, mod_api::ScreenMouseButtonReleasedPreEvent { screen: &view, button: 1, modifiers: 3 }));
    assert!(cancel!(mods, screen_mouse_scrolled_pre, &host, mod_api::ScreenMouseScrolledPreEvent { screen: &view, scroll: [1.25, -2.5], modifiers: 3 }));
    assert_eq!(LATE_CANCELS.get(), 0);
    let mut buttons = vec!["static".into()];
    for mod_ in &*mods { mod_.settings(&mut host, &mut mod_api::SettingsEvent { mod_id: HookProbe::ID, buttons: &mut buttons }); }
    assert_eq!(buttons, ["static", "first", "second"]);
    mods[0].settings_click(&mut host, &mut mod_api::SettingsClickEvent { mod_id: HookProbe::ID, button: 0 });
    assert_eq!(host.feedback, ["hooks passed"]);
    assert_eq!(batch.text, ["post", "screen", "pre"]);
}

#[derive(Default)]
struct CallbackPanic;

impl YoakeMod for CallbackPanic {
    const ID: &'static str = "snapshot-panic";
    const VERSION: &'static str = "1";
    fn client_tick(&mut self, host: &mut Host<'_>) {
        let _ = host.snapshot_client_world(|_, _, _| panic!("snapshot callback panic"));
        host.feedback("must not resume").unwrap();
    }
}

#[test]
fn callback_panics_do_not_unwind_through_c_or_disable_the_next_mod() {
    let (mods, _) = unsafe { dll_mods::in_process(Arc::from([]), &[], &[entry::<CallbackPanic>, entry::<super::Second>]).unwrap() };
    let mut host = mod_api::FakeHost { client: true, ..Default::default() };
    tick(&mods, &mut host);
    tick(&mods, &mut host);
    assert_eq!(host.feedback, ["dll-b", "dll-b"]);
}
