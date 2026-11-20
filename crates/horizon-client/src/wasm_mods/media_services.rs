//! FR-MEDIA1: `media-services` for client-full packages that declared `service.browser`.
//!
//! A package drives browsers by handle. The engine opens an installed Edge or Chrome (never another
//! program) through `html-panel`, with a profile folder of the package's own, and keeps every page on the
//! web: no `file://` pages, no downloads. Nothing starts until a package opens its first browser: no
//! threads, no processes. Frames go to the package's own `gui-texture`s; sound comes back as PCM for the
//! package's `audio-stream`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use html_panel::{Frame, Input, Options, Status};
use serde_json::Value;
use yoake_wasm::media_types::*;

use super::GameHost;

/// Browsers one package may have open at once.
pub(super) const MAX_BROWSERS: usize = 8;
/// The most pixels one browser may draw (3840 × 2160).
const MAX_PIXELS: u64 = 3840 * 2160;
const MAX_URL: usize = 8 * 1024;
const MAX_EXPRESSION: usize = 64 * 1024;
const MAX_EVAL_ANSWER: usize = 1 << 20;
const MAX_ALLOW: usize = 64;
const MAX_JPEG_BYTES: usize = 4 << 20;
/// Unanswered eval tickets one package may hold.
const MAX_TICKETS: usize = 256;

// ---------------------------------------------------------------------------------------------
// Grants: which loaded packages declared `service.browser`
// ---------------------------------------------------------------------------------------------

fn grants() -> &'static Mutex<HashSet<String>> {
    static GRANTS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    GRANTS.get_or_init(Default::default)
}

/// Record whether package `id` may use the browser services (its manifest's `service.browser`).
pub(super) fn grant(id: &str, allowed: bool) {
    let mut grants = grants().lock().unwrap_or_else(|error| error.into_inner());
    if allowed {
        grants.insert(id.to_owned());
    } else {
        grants.remove(id);
    }
}

pub(super) fn granted(id: &str) -> bool {
    grants()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains(id)
}

// ---------------------------------------------------------------------------------------------
// The page behind a handle (a real browser through html-panel, or a test double)
// ---------------------------------------------------------------------------------------------

pub(super) trait Page: Send {
    fn status(&self) -> Status;
    fn info(&self) -> html_panel::PageInfo;
    fn frame(&self) -> Option<Frame>;
    fn frames(&self) -> u64;
    fn encoded_frame(&self) -> Option<(u64, Arc<Vec<u8>>)>;
    fn set_jpeg_quality(&self, quality: u8);
    fn audio_pcm(&self) -> Vec<i16>;
    fn audio_status(&self) -> Option<Result<(), String>>;
    fn input(&self, input: &Input);
    fn set_visible(&self, visible: bool);
    fn eval_value(&self, expression: &str) -> u64;
    fn eval_result(&self, ticket: u64) -> Option<Result<String, String>>;
    fn post(&self, value: &Value) -> Result<(), String>;
    fn poll(&self) -> Vec<Value>;
    fn close(&self);
}

impl Page for html_panel::Panel {
    fn status(&self) -> Status {
        html_panel::Panel::status(self)
    }
    fn info(&self) -> html_panel::PageInfo {
        html_panel::Panel::info(self)
    }
    fn frame(&self) -> Option<Frame> {
        html_panel::Panel::frame(self)
    }
    fn frames(&self) -> u64 {
        html_panel::Panel::frames(self)
    }
    fn encoded_frame(&self) -> Option<(u64, Arc<Vec<u8>>)> {
        html_panel::Panel::encoded_frame(self)
    }
    fn set_jpeg_quality(&self, quality: u8) {
        html_panel::Panel::set_jpeg_quality(self, quality);
    }
    fn audio_pcm(&self) -> Vec<i16> {
        html_panel::Panel::audio_pcm(self)
    }
    fn audio_status(&self) -> Option<Result<(), String>> {
        html_panel::Panel::audio_status(self)
    }
    fn input(&self, input: &Input) {
        html_panel::Panel::input(self, input);
    }
    fn set_visible(&self, visible: bool) {
        html_panel::Panel::set_visible(self, visible);
    }
    fn eval_value(&self, expression: &str) -> u64 {
        html_panel::Panel::eval_value(self, expression)
    }
    fn eval_result(&self, ticket: u64) -> Option<Result<String, String>> {
        html_panel::Panel::eval_result(self, ticket)
    }
    fn post(&self, value: &Value) -> Result<(), String> {
        html_panel::Panel::post(self, value)
    }
    fn poll(&self) -> Vec<Value> {
        html_panel::Panel::poll(self)
    }
    fn close(&self) {
        html_panel::Panel::close(self);
    }
}

/// What a browser is opened with, after the package's request was checked.
pub(super) struct Request {
    pub url: String,
    pub size: [u32; 2],
    pub scale: f32,
    pub options: Options,
}

type Opener = fn(Request) -> Result<Box<dyn Page>, String>;

fn open_real(request: Request) -> Result<Box<dyn Page>, String> {
    html_panel::Panel::open_with(&request.url, request.size[0], request.size[1], request.scale, request.options)
        .map(|panel| Box::new(panel) as Box<dyn Page>)
        .map_err(|error| error.to_string())
}

fn opener() -> &'static Mutex<Opener> {
    static OPENER: OnceLock<Mutex<Opener>> = OnceLock::new();
    OPENER.get_or_init(|| Mutex::new(open_real as Opener))
}

/// Tests open a fake page instead of a browser.
#[cfg(test)]
pub(super) fn set_opener(open: Opener) {
    *opener().lock().unwrap_or_else(|error| error.into_inner()) = open;
}

// ---------------------------------------------------------------------------------------------
// One package's browsers
// ---------------------------------------------------------------------------------------------

struct Open {
    page: Box<dyn Page>,
    slot: usize,
    audio: bool,
    /// The newest frame, kept so several textures can show it.
    frame: Option<Arc<Frame>>,
    /// The last frame written to each texture.
    written: HashMap<u64, u64>,
    /// The last JPEG handed out.
    jpeg_seq: u64,
    jpeg_quality: Option<u8>,
}

#[derive(Default)]
pub(super) struct Browsers {
    open: HashMap<u32, Open>,
    next: u32,
    /// Package ticket → (handle, the page's own ticket).
    tickets: HashMap<u64, (u32, u64)>,
    next_ticket: u64,
}

impl Browsers {
    fn get(&mut self, handle: u32) -> Result<&mut Open, String> {
        self.open.get_mut(&handle).ok_or_else(|| "unknown browser handle".to_owned())
    }

    fn close(&mut self, handle: u32) -> bool {
        let Some(open) = self.open.remove(&handle) else {
            return false;
        };
        open.page.close();
        self.tickets.retain(|_, (owner, _)| *owner != handle);
        true
    }

    pub(super) fn close_all(&mut self) {
        for (_, open) in self.open.drain() {
            open.page.close();
        }
        self.tickets.clear();
    }

    fn free_slot(&self) -> usize {
        (0..MAX_BROWSERS)
            .find(|slot| !self.open.values().any(|open| open.slot == *slot))
            .unwrap_or(MAX_BROWSERS)
    }
}

impl Drop for Browsers {
    fn drop(&mut self) {
        self.close_all();
    }
}

/// The profile folder of `mod_id`'s browser in `slot`: `<game>/media-browser/<mod>/<slot>`.
fn profile(mod_id: &str, slot: usize) -> Result<PathBuf, String> {
    let safe: String = mod_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c } else { '_' })
        .collect();
    if safe.is_empty() || safe.chars().all(|c| c == '.') {
        return Err("package id cannot name a browser profile".to_owned());
    }
    let root = crate::yak_mods::game_root()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("yoake"));
    Ok(root.join("media-browser").join(safe).join(slot.to_string()))
}

pub(super) fn check_url(url: &str) -> Result<(), String> {
    if url.len() > MAX_URL {
        return Err(format!("url is longer than {MAX_URL} bytes"));
    }
    if !html_panel::web_url(url) {
        return Err("only http, https, data, blob and about:blank pages may be opened".to_owned());
    }
    Ok(())
}

fn check_allow(allow: Vec<String>) -> Result<Option<Vec<String>>, String> {
    if allow.is_empty() {
        return Ok(None);
    }
    if allow.len() > MAX_ALLOW {
        return Err(format!("at most {MAX_ALLOW} allowed prefixes"));
    }
    for prefix in &allow {
        let lower = prefix.to_ascii_lowercase();
        if prefix.len() > MAX_URL || !(lower.starts_with("https://") || lower.starts_with("http://")) {
            return Err(format!("allowed prefix {prefix:?} must start with http:// or https://"));
        }
    }
    Ok(Some(allow))
}

fn page_state(open: &Open) -> PageState {
    let info = open.page.info();
    let (status, error) = match open.page.status() {
        Status::Starting => (BrowserStatus::Starting, None),
        Status::Ready => (BrowserStatus::Ready, None),
        Status::Failed(reason) => (BrowserStatus::Failed, Some(reason)),
        Status::Closed => (BrowserStatus::Closed, None),
    };
    let audio = match (open.audio, open.page.audio_status()) {
        (false, _) => AudioStatus::Off,
        (true, None) => AudioStatus::Starting,
        (true, Some(Ok(()))) => AudioStatus::Capturing,
        (true, Some(Err(_))) => AudioStatus::Failed,
    };
    let (width, height) = open.frame.as_ref().map_or((0, 0), |frame| (frame.width, frame.height));
    PageState {
        status,
        error,
        url: info.url,
        title: info.title,
        loading: info.loading,
        frames: open.page.frames(),
        width,
        height,
        audio,
    }
}

fn input(event: InputEvent) -> Result<Input, String> {
    let finite = [event.x, event.y, event.dx, event.dy].iter().all(|value| value.is_finite());
    if !finite {
        return Err("input coordinates must be finite".to_owned());
    }
    if event.text.len() > MAX_EXPRESSION {
        return Err(format!("input text is longer than {MAX_EXPRESSION} bytes"));
    }
    let button = match event.button {
        MouseButton::None | MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
    }
    .to_owned();
    let (x, y, mods, clicks) = (event.x, event.y, event.mods & 15, event.clicks.clamp(1, 3));
    Ok(match event.kind {
        InputKind::Move => Input::Move { x, y, buttons: event.buttons & 7 },
        InputKind::Down => Input::Down { x, y, button, clicks, mods },
        InputKind::Up => Input::Up { x, y, button, clicks, mods },
        InputKind::Wheel => Input::Wheel { x, y, dx: event.dx, dy: event.dy, mods },
        InputKind::KeyDown => Input::Key { name: event.text, down: true, mods },
        InputKind::KeyUp => Input::Key { name: event.text, down: false, mods },
        InputKind::Text => Input::Type(event.text),
        InputKind::Insert => Input::Insert(event.text),
    })
}

/// Copy the part of an RGBA image that fits a texture of `limit` into it.
fn clipped(rgba: &[u8], size: [u32; 2], limit: [u32; 2]) -> Option<([u32; 2], Vec<u8>)> {
    let fit = [size[0].min(limit[0]), size[1].min(limit[1])];
    if fit[0] == 0 || fit[1] == 0 {
        return None;
    }
    if fit == size {
        return Some((fit, rgba.to_vec()));
    }
    let (row, keep) = (size[0] as usize * 4, fit[0] as usize * 4);
    let mut out = Vec::with_capacity(keep * fit[1] as usize);
    for line in rgba.chunks_exact(row).take(fit[1] as usize) {
        out.extend_from_slice(&line[..keep]);
    }
    Some((fit, out))
}

// ---------------------------------------------------------------------------------------------
// The services
// ---------------------------------------------------------------------------------------------

impl GameHost {
    fn browsers(&mut self) -> &mut Browsers {
        self.browsers.get_or_insert_with(Default::default)
    }

    fn write_rgba(&self, texture: u64, size: [u32; 2], rgba: &[u8]) -> Result<bool, String> {
        let native = self.textures.get(&texture).ok_or("invalid or foreign GUI texture")?;
        let limit = [native.texture().width(), native.texture().height()];
        let Some((fit, bytes)) = clipped(rgba, size, limit) else {
            return Ok(false);
        };
        let native = native.clone();
        self.client_call(|invocation| {
            invocation
                .host
                .read()
                .ok_or_else(|| "client host unavailable in this hook".to_owned())?
                .gui_texture_write(&native, [0, 0], fit, &bytes)
                .map_err(|error| format!("{error:?}"))
        })?;
        Ok(true)
    }
}

impl yoake_wasm::MediaServices for GameHost {
    fn browser_open(&mut self, height: u32, options: BrowserOptions, scale: f32, url: String, width: u32) -> Result<u32, String> {
        check_url(&url)?;
        let size = [width.clamp(16, 4096), height.clamp(16, 4096)];
        let scale = if scale.is_finite() { scale.clamp(0.25, 4.0) } else { 1.0 };
        let pixels = (f64::from(size[0]) * f64::from(scale)) as u64 * (f64::from(size[1]) * f64::from(scale)) as u64;
        if pixels > MAX_PIXELS {
            return Err(format!("{}x{} at scale {scale} draws more than {MAX_PIXELS} pixels", size[0], size[1]));
        }
        let allow = check_allow(options.allow)?;
        let id = self.id.clone();
        let browsers = self.browsers();
        if browsers.open.len() >= MAX_BROWSERS {
            return Err(format!("at most {MAX_BROWSERS} browsers per package"));
        }
        let slot = browsers.free_slot();
        let audio = options.audio && options.window;
        let request = Request {
            url,
            size,
            scale,
            options: Options {
                browser: None,
                profile: Some(profile(&id, slot)?),
                transparent: options.transparent,
                allow,
                watch: false,
                max_message: options.max_message.min(html_panel::DEFAULT_MAX_MESSAGE as u32) as usize,
                window: options.window,
                audio,
                jpeg: (options.jpeg_quality > 0).then(|| options.jpeg_quality.min(100)),
                web_only: true,
            },
        };
        let open = *opener().lock().unwrap_or_else(|error| error.into_inner());
        let page = open(request)?;
        let handle = loop {
            browsers.next = browsers.next.wrapping_add(1).max(1);
            if !browsers.open.contains_key(&browsers.next) {
                break browsers.next;
            }
        };
        browsers.open.insert(
            handle,
            Open { page, slot, audio, frame: None, written: HashMap::new(), jpeg_seq: 0, jpeg_quality: None },
        );
        Ok(handle)
    }

    fn browser_close(&mut self, handle: u32) -> Result<(), String> {
        if self.browsers().close(handle) { Ok(()) } else { Err("unknown browser handle".to_owned()) }
    }

    fn browser_set_visible(&mut self, handle: u32, visible: bool) -> Result<(), String> {
        self.browsers().get(handle)?.page.set_visible(visible);
        Ok(())
    }

    fn browser_navigate(&mut self, handle: u32, url: String) -> Result<(), String> {
        check_url(&url)?;
        self.browsers().get(handle)?.page.input(&Input::Nav(url));
        Ok(())
    }

    fn browser_history(&mut self, action: HistoryAction, handle: u32) -> Result<(), String> {
        let input = match action {
            HistoryAction::Back => Input::Back,
            HistoryAction::Forward => Input::Forward,
            HistoryAction::Reload => Input::Reload,
        };
        self.browsers().get(handle)?.page.input(&input);
        Ok(())
    }

    fn browser_input(&mut self, event: InputEvent, handle: u32) -> Result<(), String> {
        let input = input(event)?;
        self.browsers().get(handle)?.page.input(&input);
        Ok(())
    }

    fn browser_state(&mut self, handle: u32) -> Result<PageState, String> {
        let open = self.browsers().get(handle)?;
        if let Some(frame) = open.page.frame() {
            open.frame = Some(Arc::new(frame));
        }
        Ok(page_state(open))
    }

    fn browser_frame_to_texture(&mut self, handle: u32, texture: u64) -> Result<Option<u64>, String> {
        let open = self.browsers().get(handle)?;
        if let Some(frame) = open.page.frame() {
            open.frame = Some(Arc::new(frame));
        }
        let Some(frame) = open.frame.clone() else {
            return Ok(None);
        };
        if open.written.get(&texture) == Some(&frame.seq) {
            return Ok(None);
        }
        if !self.write_rgba(texture, [frame.width, frame.height], &frame.rgba)? {
            return Ok(None);
        }
        let open = self.browsers().get(handle)?;
        open.written.insert(texture, frame.seq);
        Ok(Some(frame.seq))
    }

    fn browser_frame_jpeg(&mut self, handle: u32, quality: u8) -> Result<Option<(u64, Vec<u8>)>, String> {
        let open = self.browsers().get(handle)?;
        let quality = quality.clamp(1, 100);
        if open.jpeg_quality != Some(quality) {
            open.page.set_jpeg_quality(quality);
            open.jpeg_quality = Some(quality);
        }
        let Some((seq, bytes)) = open.page.encoded_frame() else {
            return Ok(None);
        };
        // Until the browser sends JPEG at the new quality, the newest frame may still be PNG.
        if seq == open.jpeg_seq || !bytes.starts_with(&[0xFF, 0xD8]) {
            return Ok(None);
        }
        open.jpeg_seq = seq;
        Ok(Some((seq, bytes.as_ref().clone())))
    }

    fn jpeg_to_texture(&mut self, bytes: Vec<u8>, texture: u64) -> Result<(u32, u32), String> {
        if bytes.len() > MAX_JPEG_BYTES {
            return Err(format!("JPEG is larger than {MAX_JPEG_BYTES} bytes"));
        }
        if !self.textures.contains_key(&texture) {
            return Err("invalid or foreign GUI texture".to_owned());
        }
        let (width, height, rgba) = html_panel::decode_jpeg(&bytes)?;
        if u64::from(width) * u64::from(height) > MAX_PIXELS {
            return Err(format!("JPEG of {width}x{height} is larger than {MAX_PIXELS} pixels"));
        }
        self.write_rgba(texture, [width, height], &rgba)?;
        Ok((width, height))
    }

    fn browser_audio_pcm(&mut self, handle: u32) -> Result<Vec<i16>, String> {
        let open = self.browsers().get(handle)?;
        if !open.audio {
            return Err("this browser was opened without audio".to_owned());
        }
        Ok(open.page.audio_pcm())
    }

    fn browser_eval(&mut self, expression: String, handle: u32) -> Result<u64, String> {
        if expression.len() > MAX_EXPRESSION {
            return Err(format!("expression is longer than {MAX_EXPRESSION} bytes"));
        }
        let browsers = self.browsers();
        if browsers.tickets.len() >= MAX_TICKETS {
            return Err(format!("at most {MAX_TICKETS} unanswered evals"));
        }
        let inner = browsers.get(handle)?.page.eval_value(&expression);
        browsers.next_ticket += 1;
        let ticket = browsers.next_ticket;
        browsers.tickets.insert(ticket, (handle, inner));
        Ok(ticket)
    }

    fn browser_eval_result(&mut self, ticket: u64) -> Result<EvalAnswer, String> {
        let browsers = self.browsers();
        let &(handle, inner) = browsers.tickets.get(&ticket).ok_or("unknown or expired eval ticket")?;
        let Some(open) = browsers.open.get(&handle) else {
            browsers.tickets.remove(&ticket);
            return Err("the browser was closed".to_owned());
        };
        let Some(answer) = open.page.eval_result(inner) else {
            return Ok(EvalAnswer { done: false, ok: false, value: String::new() });
        };
        browsers.tickets.remove(&ticket);
        Ok(match answer {
            Ok(value) if value.len() > MAX_EVAL_ANSWER => {
                EvalAnswer { done: true, ok: false, value: format!("answer is longer than {MAX_EVAL_ANSWER} bytes") }
            }
            Ok(value) => EvalAnswer { done: true, ok: true, value },
            Err(error) => EvalAnswer { done: true, ok: false, value: error },
        })
    }

    fn browser_post(&mut self, handle: u32, json: String) -> Result<(), String> {
        let value: Value = serde_json::from_str(&json).map_err(|error| format!("not JSON: {error}"))?;
        self.browsers().get(handle)?.page.post(&value)
    }

    fn browser_messages(&mut self, handle: u32) -> Result<Vec<String>, String> {
        Ok(self.browsers().get(handle)?.page.poll().iter().map(Value::to_string).collect())
    }
}

#[cfg(test)]
#[path = "media_services/tests.rs"]
mod tests;
