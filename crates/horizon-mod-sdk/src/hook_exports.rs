use crate::{abi, exports, Flow, Host, HudBatch, NetFrameReceived, NetState, ScreenView, SettingsButtons, YoakeMod};
use abi::{Handle, HookResult, Status};
use std::{ffi::c_void, marker::PhantomData, time::Duration};

pub const fn hooks<Mod: YoakeMod>() -> abi::HookTableV1 {
    abi::HookTableV1 {
        header: abi::TableHeader::for_type::<abi::HookTableV1>(),
        client_tick: Some(exports::client_tick::<Mod>),
        key_input: Some(exports::key_input::<Mod>),
        hud: Some(exports::hud::<Mod>),
        mod_net_receive: Some(exports::mod_net_receive::<Mod>),
        frame: Some(frame::<Mod>),
        local_player_tick_end: Some(local_player_tick_end::<Mod>),
        net_frames_listened: Some(net_frames_listened::<Mod>),
        net_frame_received: Some(net_frame_received::<Mod>),
        net_session_ended: Some(net_session_ended::<Mod>),
        gui_layer_pre: Some(gui_layer_pre::<Mod>),
        gui_layer_post: Some(gui_layer_post::<Mod>),
        screen_render_post: Some(screen_render_post::<Mod>),
        screen_key_pressed_pre: Some(screen_key_pressed_pre::<Mod>),
        screen_char_typed_pre: Some(screen_char_typed_pre::<Mod>),
        screen_mouse_button_pressed_pre: Some(screen_mouse_button_pressed_pre::<Mod>),
        screen_mouse_button_released_pre: Some(screen_mouse_button_released_pre::<Mod>),
        screen_mouse_scrolled_pre: Some(screen_mouse_scrolled_pre::<Mod>),
        settings: Some(settings::<Mod>),
        settings_click: Some(settings_click::<Mod>),
    }
}

fn batch<'call>(host: &Host<'_>, handle: Handle) -> Result<HudBatch<'call>, Status> {
    if !handle.is_valid() { return Err(Status::INVALID); }
    Ok(HudBatch { handle, host: host.handle, table: host.table, lifetime: PhantomData, thread: PhantomData })
}

unsafe fn screen<'call>(view: &abi::ScreenView) -> Result<ScreenView<'call>, Status> {
    if ![view.rect.x, view.rect.y, view.rect.width, view.rect.height, view.mouse.x, view.mouse.y, view.partial_tick].iter().all(|value| value.is_finite()) {
        return Err(Status::INVALID);
    }
    Ok(ScreenView {
        id: unsafe { view.id.as_str(abi::MAX_TEXT_BYTES)? },
        rect: [view.rect.x, view.rect.y, view.rect.width, view.rect.height],
        mouse: [view.mouse.x, view.mouse.y], partial_tick: view.partial_tick,
    })
}

unsafe extern "C" fn frame<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::FrameEvent) -> HookResult {
    unsafe { exports::invoke::<Mod>(instance, handle, |value, host| {
        value.frame(host, Duration::from_nanos(exports::event(input)?.elapsed_ns));
        Ok(())
    }) }
}

unsafe extern "C" fn local_player_tick_end<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::TickEvent) -> HookResult {
    unsafe { exports::invoke::<Mod>(instance, handle, |value, host| {
        if exports::event(input)?.reserved != 0 { return Err(Status::INVALID); }
        value.local_player_tick_end(host);
        Ok(())
    }) }
}

unsafe extern "C" fn net_frames_listened<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::NetFramesListenedEvent) -> HookResult {
    unsafe { exports::invoke::<Mod>(instance, handle, |value, _host| {
        let output = exports::event(input)?.listened;
        if output.is_null() || !output.is_aligned() || *output > 1 { return Err(Status::INVALID); }
        let previous = *output != 0;
        let mut listened = previous;
        value.net_frames_listened(&mut listened);
        *output = u32::from(previous || listened);
        Ok(())
    }) }
}

unsafe extern "C" fn net_frame_received<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::NetFrameReceived) -> HookResult {
    unsafe { exports::invoke::<Mod>(instance, handle, |value, _host| {
        let input = exports::event(input)?;
        value.net_frame_received(NetFrameReceived {
            state: NetState::try_from(input.state)?, packet_id: input.packet_id,
            body: input.body.as_slice(abi::MAX_FRAME_BYTES)?, monotonic_ns: input.monotonic_ns, session: input.session,
        });
        Ok(())
    }) }
}

unsafe extern "C" fn net_session_ended<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::NetSessionEndedEvent) -> HookResult {
    unsafe { exports::invoke::<Mod>(instance, handle, |value, _host| {
        let input = exports::event(input)?;
        value.net_session_ended(input.session, input.reason.as_str(abi::MAX_TEXT_BYTES)?);
        Ok(())
    }) }
}

unsafe extern "C" fn gui_layer_pre<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::GuiLayerEvent) -> HookResult {
    unsafe { exports::invoke_flow::<Mod>(instance, handle, |value, host| {
        let input = exports::event(input)?;
        let drawing = batch(host, input.batch)?;
        Ok(value.gui_layer_pre(host, input.layer.as_str(abi::MAX_TEXT_BYTES)?, drawing))
    }) }
}

unsafe extern "C" fn gui_layer_post<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::GuiLayerEvent) -> HookResult {
    unsafe { exports::invoke::<Mod>(instance, handle, |value, host| {
        let input = exports::event(input)?;
        let drawing = batch(host, input.batch)?;
        value.gui_layer_post(host, input.layer.as_str(abi::MAX_TEXT_BYTES)?, drawing);
        Ok(())
    }) }
}

unsafe extern "C" fn screen_render_post<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::ScreenRenderEvent) -> HookResult {
    unsafe { exports::invoke::<Mod>(instance, handle, |value, host| {
        let input = exports::event(input)?;
        let drawing = batch(host, input.batch)?;
        value.screen_render_post(host, screen(&input.screen)?, drawing);
        Ok(())
    }) }
}

unsafe extern "C" fn screen_key_pressed_pre<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::ScreenKeyEvent) -> HookResult {
    unsafe { exports::invoke_flow::<Mod>(instance, handle, |value, host| {
        let input = exports::event(input)?;
        if input.repeat > 1 { return Err(Status::INVALID); }
        Ok(value.screen_key_pressed_pre(host, screen(&input.screen)?, input.key.as_str(abi::MAX_TEXT_BYTES)?, input.repeat != 0, input.modifiers))
    }) }
}

unsafe extern "C" fn screen_char_typed_pre<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::ScreenCharEvent) -> HookResult {
    unsafe { exports::invoke_flow::<Mod>(instance, handle, |value, host| {
        let input = exports::event(input)?;
        let character = char::from_u32(input.character).ok_or(Status::INVALID)?;
        Ok(value.screen_char_typed_pre(host, screen(&input.screen)?, character, input.modifiers))
    }) }
}

macro_rules! mouse_button {
    ($method:ident) => {
        unsafe extern "C" fn $method<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::ScreenMouseButtonEvent) -> HookResult {
            unsafe { exports::invoke_flow::<Mod>(instance, handle, |value, host| {
                let input = exports::event(input)?;
                Ok(value.$method(host, screen(&input.screen)?, input.button, input.modifiers))
            }) }
        }
    };
}
mouse_button!(screen_mouse_button_pressed_pre);
mouse_button!(screen_mouse_button_released_pre);

unsafe extern "C" fn screen_mouse_scrolled_pre<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::ScreenScrollEvent) -> HookResult {
    unsafe { exports::invoke_flow::<Mod>(instance, handle, |value, host| {
        let input = exports::event(input)?;
        if !input.scroll.iter().all(|value| value.is_finite()) { return Err(Status::INVALID); }
        Ok(value.screen_mouse_scrolled_pre(host, screen(&input.screen)?, input.scroll, input.modifiers))
    }) }
}

unsafe extern "C" fn settings<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::SettingsEvent) -> HookResult {
    unsafe { exports::invoke::<Mod>(instance, handle, |value, host| {
        let input = exports::event(input)?;
        let mut buttons = SettingsButtons { push: Some(input.push.ok_or(Status::INVALID)?), user: input.user, lifetime: PhantomData };
        value.settings(host, input.mod_id.as_str(abi::MAX_TEXT_BYTES)?, &mut buttons);
        Ok(())
    }) }
}

unsafe extern "C" fn settings_click<Mod: YoakeMod>(instance: *mut c_void, handle: Handle, input: *const abi::SettingsClickEvent) -> HookResult {
    unsafe { exports::invoke::<Mod>(instance, handle, |value, host| {
        let input = exports::event(input)?;
        value.settings_click(host, input.mod_id.as_str(abi::MAX_TEXT_BYTES)?, input.button);
        Ok(())
    }) }
}
