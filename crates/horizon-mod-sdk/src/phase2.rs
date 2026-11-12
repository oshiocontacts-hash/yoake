use super::*;
use std::{any::Any, panic::{catch_unwind, resume_unwind}};

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow { Continue = abi::FLOW_CONTINUE, Cancel = abi::FLOW_CANCEL }

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetState { Login = abi::NET_LOGIN, Configuration = abi::NET_CONFIGURATION, Play = abi::NET_PLAY }

impl TryFrom<u32> for NetState {
    type Error = Status;
    fn try_from(value: u32) -> Result<Self, Status> {
        match value {
            abi::NET_LOGIN => Ok(Self::Login),
            abi::NET_CONFIGURATION => Ok(Self::Configuration),
            abi::NET_PLAY => Ok(Self::Play),
            _ => Err(Status::INVALID),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NetFrameReceived<'call> {
    pub state: NetState,
    pub packet_id: i32,
    pub body: &'call [u8],
    pub monotonic_ns: u64,
    pub session: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct ScreenView<'call> {
    pub id: &'call str,
    pub rect: [f32; 4],
    pub mouse: [f32; 2],
    pub partial_tick: f32,
}

#[derive(Clone, Debug)]
pub struct LocalPlayerSnapshot {
    pub entity_id: i32,
    pub uuid: u128,
    pub pos: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub head_yaw: f32,
    pub body_yaw: f32,
    pub flags: u32,
    pub pose: u32,
    pub swing_hand: u32,
    pub swing_ticks: u32,
    pub using_hand: u32,
    pub use_ticks: i32,
    pub equipment: [Vec<u8>; 6],
    pub vehicle: Option<i32>,
    pub skin_parts: u32,
    pub dimension: String,
    pub game_mode: u32,
    pub health: f32,
    pub max_health: Option<f32>,
    pub food_level: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotReport { pub written: usize, pub missing: Vec<String> }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account { pub name: String, pub uuid: u128, pub online: bool }

pub type GuiNineSlice = abi::GuiNineSlice;

#[derive(Debug)]
pub struct GuiTexture { handle: Handle }

pub struct SettingsButtons<'call> {
    pub(super) push: abi::SettingsPush,
    pub(super) user: *mut c_void,
    pub(super) lifetime: PhantomData<&'call mut ()>,
}

impl SettingsButtons<'_> {
    pub fn push(&mut self, label: &str) -> Result<(), Status> {
        let callback = self.push.ok_or(Status::UNSUPPORTED)?;
        ok(unsafe { callback(self.user, StrView::new(label)) })
    }
}

pub(super) fn rect(value: [f32; 4]) -> abi::Rect {
    abi::Rect { x: value[0], y: value[1], width: value[2], height: value[3] }
}

impl HudBatch<'_> {
    pub fn len(&self) -> Result<usize, Status> {
        let service = self.table.batch_len.ok_or(Status::UNSUPPORTED)?;
        let mut length = 0;
        ok(unsafe { service(self.host, self.handle, &mut length) })?;
        Ok(length)
    }

    pub fn is_empty(&self) -> Result<bool, Status> { self.len().map(|length| length == 0) }

    pub fn scale_since(&mut self, start: usize, center_x: f32, center_y: f32, scale_x: f32, scale_y: f32) -> Result<(), Status> {
        let service = self.table.batch_scale_since.ok_or(Status::UNSUPPORTED)?;
        ok(unsafe { service(self.host, self.handle, start, abi::Point { x: center_x, y: center_y }, abi::Point { x: scale_x, y: scale_y }) })
    }

    pub fn clip_since(&mut self, start: usize, left: f32, top: f32, right: f32, bottom: f32) -> Result<(), Status> {
        let service = self.table.batch_clip_since.ok_or(Status::UNSUPPORTED)?;
        ok(unsafe { service(self.host, self.handle, start, rect([left, top, right - left, bottom - top])) })
    }

    pub fn tint_since(&mut self, start: usize, alpha: f32) -> Result<(), Status> {
        let service = self.table.batch_tint_since.ok_or(Status::UNSUPPORTED)?;
        ok(unsafe { service(self.host, self.handle, start, alpha) })
    }
}

type Panic = Box<dyn Any + Send>;

struct CallbackValue<Value> { value: Option<Value>, panic: Option<Panic> }

impl<Value> Default for CallbackValue<Value> {
    fn default() -> Self { Self { value: None, panic: None } }
}

impl<Value> CallbackValue<Value> {
    fn finish(self, status: Status) -> Result<Option<Value>, Status> {
        if let Some(payload) = self.panic { resume_unwind(payload); }
        if status == Status::NOT_FOUND { return Ok(None); }
        ok(status)?;
        self.value.map(Some).ok_or(Status::INVALID)
    }

    fn receive(&mut self, operation: impl FnOnce() -> Result<Value, Status>) -> Status {
        if self.value.is_some() || self.panic.is_some() { return Status::INVALID; }
        match catch_unwind(AssertUnwindSafe(operation)) {
            Ok(Ok(value)) => { self.value = Some(value); Status::OK }
            Ok(Err(status)) => status,
            Err(payload) => { self.panic = Some(payload); Status::PANIC }
        }
    }
}

unsafe extern "C" fn receive_player(user: *mut c_void, pointer: *const abi::LocalPlayerSnapshot) -> Status {
    let state = unsafe { &mut *user.cast::<CallbackValue<LocalPlayerSnapshot>>() };
    state.receive(|| {
        let value = unsafe { exports::event(pointer)? };
        let mut equipment = std::array::from_fn(|_| Vec::new());
        for (destination, source) in equipment.iter_mut().zip(value.equipment) {
            *destination = unsafe { source.as_slice(abi::MAX_FRAME_BYTES)? }.to_vec();
        }
        Ok(LocalPlayerSnapshot {
            entity_id: value.entity_id, uuid: u128::from_be_bytes(value.uuid), pos: value.pos,
            yaw: value.yaw, pitch: value.pitch, head_yaw: value.head_yaw, body_yaw: value.body_yaw,
            flags: value.flags, pose: value.pose, swing_hand: value.swing_hand, swing_ticks: value.swing_ticks,
            using_hand: value.using_hand, use_ticks: value.use_ticks, equipment,
            vehicle: (value.flags & abi::PLAYER_HAS_VEHICLE != 0).then_some(value.vehicle),
            skin_parts: value.skin_parts, dimension: unsafe { value.dimension.as_str(abi::MAX_TEXT_BYTES)? }.to_owned(),
            game_mode: value.game_mode, health: value.health,
            max_health: (value.flags & abi::PLAYER_HAS_MAX_HEALTH != 0).then_some(value.max_health), food_level: value.food_level,
        })
    })
}

unsafe extern "C" fn receive_account(user: *mut c_void, pointer: *const abi::Account) -> Status {
    let state = unsafe { &mut *user.cast::<CallbackValue<Account>>() };
    state.receive(|| {
        let value = unsafe { exports::event(pointer)? };
        if value.online > 1 { return Err(Status::INVALID); }
        Ok(Account { name: unsafe { value.name.as_str(abi::MAX_TEXT_BYTES)? }.to_owned(), uuid: u128::from_be_bytes(value.uuid), online: value.online != 0 })
    })
}

struct SnapshotState<'call> {
    callback: &'call mut dyn FnMut(NetState, i32, &[u8]),
    result: CallbackValue<SnapshotReport>,
    error: Option<Status>,
}

unsafe extern "C" fn receive_frame(user: *mut c_void, state: u32, packet_id: i32, body: abi::ByteView) -> Status {
    let context = unsafe { &mut *user.cast::<SnapshotState<'_>>() };
    if let Some(status) = context.error { return status; }
    match catch_unwind(AssertUnwindSafe(|| {
        let state = NetState::try_from(state)?;
        let body = unsafe { body.as_slice(abi::MAX_FRAME_BYTES)? };
        (context.callback)(state, packet_id, body);
        Ok::<_, Status>(())
    })) {
        Ok(Ok(())) => Status::OK,
        Ok(Err(status)) => { context.error = Some(status); status }
        Err(payload) => { context.result.panic = Some(payload); context.error = Some(Status::PANIC); Status::PANIC }
    }
}

unsafe extern "C" fn receive_report(user: *mut c_void, pointer: *const abi::SnapshotReport) -> Status {
    let context = unsafe { &mut *user.cast::<SnapshotState<'_>>() };
    if let Some(status) = context.error { return status; }
    context.result.receive(|| {
        let value = unsafe { exports::event(pointer)? };
        if value.missing_len > abi::MAX_SETTINGS_BUTTONS || (value.missing_len != 0 && (value.missing.is_null() || !value.missing.is_aligned())) { return Err(Status::INVALID); }
        let missing = if value.missing_len == 0 { &[][..] } else { unsafe { std::slice::from_raw_parts(value.missing, value.missing_len) } };
        let missing = missing.iter().map(|text| unsafe { text.as_str(abi::MAX_TEXT_BYTES) }.map(str::to_owned)).collect::<Result<_, _>>()?;
        Ok(SnapshotReport { written: value.written, missing })
    })
}

impl Host<'_> {
    pub fn local_player_snapshot(&self) -> Result<Option<LocalPlayerSnapshot>, Status> {
        let service = self.table.local_player_snapshot.ok_or(Status::UNSUPPORTED)?;
        let mut result = CallbackValue::<LocalPlayerSnapshot>::default();
        let status = unsafe { service(self.handle, Some(receive_player), (&mut result as *mut CallbackValue<_>).cast()) };
        result.finish(status)
    }

    pub fn account(&self) -> Result<Option<Account>, Status> {
        let service = self.table.account.ok_or(Status::UNSUPPORTED)?;
        let mut result = CallbackValue::<Account>::default();
        let status = unsafe { service(self.handle, Some(receive_account), (&mut result as *mut CallbackValue<_>).cast()) };
        result.finish(status)
    }

    pub fn snapshot_client_world(&mut self, mut callback: impl FnMut(NetState, i32, &[u8])) -> Result<SnapshotReport, Status> {
        let service = self.table.snapshot_client_world.ok_or(Status::UNSUPPORTED)?;
        let mut context = SnapshotState { callback: &mut callback, result: CallbackValue::default(), error: None };
        let status = unsafe { service(self.handle, Some(receive_frame), Some(receive_report), (&mut context as *mut SnapshotState<'_>).cast()) };
        context.result.finish(context.error.unwrap_or(status))?.ok_or(Status::INVALID)
    }

    pub fn playing(&self) -> Result<bool, Status> {
        let service = self.table.playing.ok_or(Status::UNSUPPORTED)?;
        let mut value = 0;
        ok(unsafe { service(self.handle, &mut value) })?;
        if value > 1 { return Err(Status::INVALID); }
        Ok(value != 0)
    }

    pub fn text_width(&self, text: &str) -> Result<Option<f32>, Status> {
        let service = self.table.text_width.ok_or(Status::UNSUPPORTED)?;
        let mut width = 0.0;
        let status = unsafe { service(self.handle, StrView::new(text), &mut width) };
        if status == Status::NOT_FOUND { return Ok(None); }
        ok(status)?;
        if !width.is_finite() { return Err(Status::INVALID); }
        Ok(Some(width))
    }

    pub fn instance_dir(&self) -> Result<std::path::PathBuf, Status> {
        let service = self.table.instance_dir.ok_or(Status::UNSUPPORTED)?;
        read_output(|output| unsafe { service(self.handle, output) })?.map(std::path::PathBuf::from).ok_or(Status::NOT_FOUND)
    }

    pub fn mod_screen(&self) -> Result<Option<String>, Status> {
        let service = self.table.mod_screen.ok_or(Status::UNSUPPORTED)?;
        read_output(|output| unsafe { service(self.handle, output) })
    }

    pub fn mod_screen_open(&mut self, mod_id: &str) -> Result<bool, Status> { self.open_screen(mod_id, false) }
    pub fn mod_screen_open_anywhere(&mut self, mod_id: &str) -> Result<bool, Status> { self.open_screen(mod_id, true) }

    fn open_screen(&mut self, mod_id: &str, anywhere: bool) -> Result<bool, Status> {
        let service = if anywhere { self.table.mod_screen_open_anywhere } else { self.table.mod_screen_open }.ok_or(Status::UNSUPPORTED)?;
        let mut opened = 0;
        ok(unsafe { service(self.handle, StrView::new(mod_id), &mut opened) })?;
        if opened > 1 { return Err(Status::INVALID); }
        Ok(opened != 0)
    }

    pub fn mod_screen_close(&mut self, mod_id: &str) -> Result<(), Status> {
        let service = self.table.mod_screen_close.ok_or(Status::UNSUPPORTED)?;
        ok(unsafe { service(self.handle, StrView::new(mod_id)) })
    }

    pub fn fill(&self, batch: &mut HudBatch<'_>, bounds: [f32; 4], argb: u32) -> Result<(), Status> {
        let service = self.table.hud_rect.ok_or(Status::UNSUPPORTED)?;
        ok(unsafe { service(self.handle, batch.handle, rect(bounds), argb) })
    }

    pub fn text(&self, batch: &mut HudBatch<'_>, text: &str, at: [f32; 2], rgb: u32, shadow: bool) -> Result<(), Status> {
        let service = self.table.hud_text.ok_or(Status::UNSUPPORTED)?;
        ok(unsafe { service(self.handle, batch.handle, StrView::new(text), abi::Point { x: at[0], y: at[1] }, rgb, u32::from(shadow)) })
    }

    pub fn gui_texture_create(&self, size: [u32; 2], format: u32, filter: u32) -> Result<GuiTexture, Status> {
        let service = self.table.gui_texture_create.ok_or(Status::UNSUPPORTED)?;
        let mut handle = Handle::default();
        ok(unsafe { service(self.handle, size[0], size[1], format, filter, &mut handle) })?;
        if !handle.is_valid() { return Err(Status::INVALID); }
        Ok(GuiTexture { handle })
    }

    pub fn gui_texture_release(&self, texture: GuiTexture) -> Result<(), Status> {
        let service = self.table.gui_texture_release.ok_or(Status::UNSUPPORTED)?;
        ok(unsafe { service(self.handle, texture.handle) })
    }

    pub fn gui_texture(&self, batch: &mut HudBatch<'_>, texture: &GuiTexture, bounds: [f32; 4], uv: [f32; 4], argb: u32, nine_slice: Option<GuiNineSlice>) -> Result<bool, Status> {
        let service = self.table.gui_texture.ok_or(Status::UNSUPPORTED)?;
        let mut drawn = 0;
        let nine_slice = nine_slice.as_ref().map_or(std::ptr::null(), |value| value as *const _);
        ok(unsafe { service(self.handle, batch.handle, texture.handle, rect(bounds), rect(uv), argb, nine_slice, &mut drawn) })?;
        if drawn > 1 { return Err(Status::INVALID); }
        Ok(drawn != 0)
    }
}
