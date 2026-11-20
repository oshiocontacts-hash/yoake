//! FR-MEDIA1 tests. They need no browser: pages are test doubles opened through `set_opener`.

use std::sync::{Arc, Mutex};

use html_panel::{Frame, Input, PageInfo, Status};
use serde_json::{Value, json};
use yoake_wasm::MediaServices;

use super::*;

#[derive(Default)]
struct FakeState {
    profile: PathBuf,
    url: String,
    web_only: bool,
    audio: bool,
    allow: Option<Vec<String>>,
    inputs: Vec<Input>,
    visible: Vec<bool>,
    posts: Vec<Value>,
    closed: bool,
    quality: Vec<u8>,
}

struct Fake(Arc<Mutex<FakeState>>);

fn opened() -> &'static Mutex<Vec<Arc<Mutex<FakeState>>>> {
    static OPENED: Mutex<Vec<Arc<Mutex<FakeState>>>> = Mutex::new(Vec::new());
    &OPENED
}

fn fake_open(request: Request) -> Result<Box<dyn Page>, String> {
    let state = Arc::new(Mutex::new(FakeState {
        profile: request.options.profile.clone().unwrap_or_default(),
        url: request.url,
        web_only: request.options.web_only,
        audio: request.options.audio,
        allow: request.options.allow,
        ..FakeState::default()
    }));
    opened().lock().unwrap().push(Arc::clone(&state));
    Ok(Box::new(Fake(state)))
}

/// The fakes `mod_id` opened, oldest first.
fn fakes(mod_id: &str) -> Vec<Arc<Mutex<FakeState>>> {
    opened()
        .lock()
        .unwrap()
        .iter()
        .filter(|state| state.lock().unwrap().profile.components().any(|part| part.as_os_str() == mod_id))
        .cloned()
        .collect()
}

impl Page for Fake {
    fn status(&self) -> Status {
        if self.0.lock().unwrap().closed { Status::Closed } else { Status::Ready }
    }
    fn info(&self) -> PageInfo {
        PageInfo { url: self.0.lock().unwrap().url.clone(), title: "Fake".into(), loading: false }
    }
    fn frame(&self) -> Option<Frame> {
        Some(Frame { width: 2, height: 1, rgba: vec![255; 8], seq: 7 })
    }
    fn frames(&self) -> u64 {
        7
    }
    fn encoded_frame(&self) -> Option<(u64, Arc<Vec<u8>>)> {
        Some((9, Arc::new(vec![0xFF, 0xD8, 0xFF, 0xD9])))
    }
    fn set_jpeg_quality(&self, quality: u8) {
        self.0.lock().unwrap().quality.push(quality);
    }
    fn audio_pcm(&self) -> Vec<i16> {
        vec![1, -1, 2, -2]
    }
    fn audio_status(&self) -> Option<Result<(), String>> {
        Some(Ok(()))
    }
    fn input(&self, input: &Input) {
        let mut state = self.0.lock().unwrap();
        if let Input::Nav(url) = input {
            state.url = url.clone();
        }
        state.inputs.push(input.clone());
    }
    fn set_visible(&self, visible: bool) {
        self.0.lock().unwrap().visible.push(visible);
    }
    fn eval_value(&self, expression: &str) -> u64 {
        if expression == "slow" { 2 } else { 1 }
    }
    fn eval_result(&self, ticket: u64) -> Option<Result<String, String>> {
        (ticket == 1).then(|| Ok("42".to_owned()))
    }
    fn post(&self, value: &Value) -> Result<(), String> {
        self.0.lock().unwrap().posts.push(value.clone());
        Ok(())
    }
    fn poll(&self) -> Vec<Value> {
        vec![json!({ "hello": 1 })]
    }
    fn close(&self) {
        self.0.lock().unwrap().closed = true;
    }
}

fn options() -> BrowserOptions {
    BrowserOptions { allow: Vec::new(), audio: false, jpeg_quality: 0, max_message: 0, transparent: false, window: false }
}

fn host(id: &str) -> GameHost {
    set_opener(fake_open);
    GameHost::new(id)
}

#[test]
fn nothing_starts_until_a_browser_is_opened() {
    let host = host("media-idle");
    assert!(host.browsers.is_none());
    assert!(fakes("media-idle").is_empty());
}

#[test]
fn the_permission_gates_the_services() {
    let mut host = host("media-grant");
    assert!(yoake_wasm::ClientHost::media_services(&mut host).is_none());
    grant("media-grant", true);
    assert!(yoake_wasm::ClientHost::media_services(&mut host).is_some());
    grant("media-grant", false);
    assert!(yoake_wasm::ClientHost::media_services(&mut host).is_none());
}

#[test]
fn opens_with_a_profile_of_the_package_and_web_only() {
    let mut host = host("media-open");
    let first = host.browser_open(720, options(), 1.0, "https://example.com/".into(), 1280).unwrap();
    let second = host.browser_open(720, options(), 1.0, "about:blank".into(), 1280).unwrap();
    assert_ne!(first, second);
    let opened = fakes("media-open");
    assert_eq!(opened.len(), 2);
    let (a, b) = (opened[0].lock().unwrap(), opened[1].lock().unwrap());
    assert!(a.web_only && b.web_only);
    assert!(a.profile.ends_with("media-browser/media-open/0") || a.profile.ends_with("media-browser\\media-open\\0"));
    assert_ne!(a.profile, b.profile, "two browsers of one package never share a profile");
    assert_eq!(a.allow, None);
}

#[test]
fn refuses_local_files_and_other_schemes() {
    let mut host = host("media-schemes");
    for url in ["file:///C:/Users/", "chrome://settings", "edge://flags", "javascript:alert(1)", "view-source:https://a", "about:config"] {
        assert!(host.browser_open(100, options(), 1.0, url.into(), 100).is_err(), "{url}");
    }
    let handle = host.browser_open(100, options(), 1.0, "data:text/html,hi".into(), 100).unwrap();
    assert!(host.browser_navigate(handle, "FILE:///etc/passwd".into()).is_err());
    assert!(host.browser_navigate(handle, "https://example.com/".into()).is_ok());
    let mut bad = options();
    bad.allow = vec!["file:///".into()];
    assert!(host.browser_open(100, bad, 1.0, "https://a/".into(), 100).is_err());
}

#[test]
fn limits_browsers_and_pixels() {
    let mut host = host("media-limits");
    assert!(host.browser_open(4096, options(), 4.0, "https://a/".into(), 4096).is_err());
    for _ in 0..MAX_BROWSERS {
        host.browser_open(100, options(), 1.0, "https://a/".into(), 100).unwrap();
    }
    assert!(host.browser_open(100, options(), 1.0, "https://a/".into(), 100).is_err());
}

#[test]
fn input_history_visibility_and_state() {
    let mut host = host("media-input");
    let handle = host.browser_open(100, options(), 1.0, "https://a/".into(), 100).unwrap();
    let event = InputEvent {
        kind: InputKind::Down,
        x: 3.0,
        y: 4.0,
        dx: 0.0,
        dy: 0.0,
        button: MouseButton::Right,
        buttons: 2,
        clicks: 1,
        mods: 2,
        text: String::new(),
    };
    host.browser_input(event.clone(), handle).unwrap();
    assert!(host.browser_input(InputEvent { x: f64::NAN, ..event }, handle).is_err());
    host.browser_history(HistoryAction::Back, handle).unwrap();
    host.browser_set_visible(handle, false).unwrap();
    let state = host.browser_state(handle).unwrap();
    assert_eq!((state.status, state.url.as_str(), state.width, state.height), (BrowserStatus::Ready, "https://a/", 2, 1));
    assert_eq!(state.audio, AudioStatus::Off);
    let fake = fakes("media-input").pop().unwrap();
    let fake = fake.lock().unwrap();
    assert_eq!(
        fake.inputs,
        vec![
            Input::Down { x: 3.0, y: 4.0, button: "right".into(), clicks: 1, mods: 2 },
            Input::Back,
        ]
    );
    assert_eq!(fake.visible, vec![false]);
    assert!(host.browser_input(InputEvent { kind: InputKind::Move, ..fake_event() }, 999).is_err());
}

fn fake_event() -> InputEvent {
    InputEvent { kind: InputKind::Move, x: 0.0, y: 0.0, dx: 0.0, dy: 0.0, button: MouseButton::None, buttons: 0, clicks: 0, mods: 0, text: String::new() }
}

#[test]
fn audio_only_with_a_window() {
    let mut host = host("media-audio");
    let mut asked = options();
    asked.audio = true;
    let headless = host.browser_open(100, asked.clone(), 1.0, "https://a/".into(), 100).unwrap();
    assert!(host.browser_audio_pcm(headless).is_err());
    asked.window = true;
    let windowed = host.browser_open(100, asked, 1.0, "https://a/".into(), 100).unwrap();
    assert_eq!(host.browser_audio_pcm(windowed).unwrap(), vec![1, -1, 2, -2]);
    assert_eq!(host.browser_state(windowed).unwrap().audio, AudioStatus::Capturing);
    assert!(fakes("media-audio")[1].lock().unwrap().audio);
}

#[test]
fn jpeg_frames_once_each() {
    let mut host = host("media-jpeg");
    let handle = host.browser_open(100, options(), 1.0, "https://a/".into(), 100).unwrap();
    assert_eq!(host.browser_frame_jpeg(handle, 60).unwrap(), Some((9, vec![0xFF, 0xD8, 0xFF, 0xD9])));
    assert_eq!(host.browser_frame_jpeg(handle, 60).unwrap(), None, "the same frame is not sent twice");
    assert_eq!(fakes("media-jpeg")[0].lock().unwrap().quality, vec![60]);
}

#[test]
fn eval_tickets_belong_to_their_browser() {
    let mut host = host("media-eval");
    let handle = host.browser_open(100, options(), 1.0, "https://a/".into(), 100).unwrap();
    let fast = host.browser_eval("1+41".into(), handle).unwrap();
    let slow = host.browser_eval("slow".into(), handle).unwrap();
    let pending = host.browser_eval_result(slow).unwrap();
    assert!(!pending.done);
    let answer = host.browser_eval_result(fast).unwrap();
    assert_eq!((answer.done, answer.ok, answer.value.as_str()), (true, true, "42"));
    assert!(host.browser_eval_result(fast).is_err(), "an answer is handed out once");
    host.browser_close(handle).unwrap();
    assert!(host.browser_eval_result(slow).is_err(), "closing drops the browser's tickets");
    assert!(host.browser_eval("x".repeat(MAX_EXPRESSION + 1), 1).is_err());
}

#[test]
fn post_and_messages() {
    let mut host = host("media-post");
    let handle = host.browser_open(100, options(), 1.0, "https://a/".into(), 100).unwrap();
    host.browser_post(handle, r#"{"a":1}"#.into()).unwrap();
    assert!(host.browser_post(handle, "{".into()).is_err());
    assert_eq!(host.browser_messages(handle).unwrap(), vec![r#"{"hello":1}"#.to_owned()]);
    assert_eq!(fakes("media-post")[0].lock().unwrap().posts, vec![json!({ "a": 1 })]);
}

#[test]
fn unloading_closes_every_browser() {
    let mut host = host("media-unload");
    host.browser_open(100, options(), 1.0, "https://a/".into(), 100).unwrap();
    host.browser_open(100, options(), 1.0, "https://b/".into(), 100).unwrap();
    yoake_wasm::ClientHost::release_resources(&mut host);
    assert!(fakes("media-unload").iter().all(|fake| fake.lock().unwrap().closed));
    assert!(host.browsers.as_ref().is_none_or(|browsers| browsers.open.is_empty()));
}

#[test]
fn clipping_keeps_the_top_left() {
    let rgba: Vec<u8> = (0..4 * 3 * 2).map(|value| value as u8).collect();
    let (fit, bytes) = clipped(&rgba, [3, 2], [2, 1]).unwrap();
    assert_eq!(fit, [2, 1]);
    assert_eq!(bytes, (0..8).collect::<Vec<u8>>());
    assert!(clipped(&rgba, [3, 2], [0, 4]).is_none());
}

/// Opens the installed Edge or Chrome for real (run with `--ignored` on a PC that has one).
#[test]
#[ignore = "needs an installed Edge or Chrome"]
fn real_browser_draws_a_page() {
    let page = open_real(Request {
        url: "data:text/html,<body style='background:%23f00'>hi</body>".into(),
        size: [64, 48],
        scale: 1.0,
        options: html_panel::Options { web_only: true, ..Default::default() },
    })
    .expect("a browser is installed");
    let started = std::time::Instant::now();
    let frame = loop {
        if let Some(frame) = page.frame() {
            break frame;
        }
        if let Status::Failed(reason) = page.status() {
            panic!("browser failed: {reason}");
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(30), "no frame in 30 s");
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert_eq!((frame.width, frame.height), (64, 48));
    assert_eq!(&frame.rgba[..3], &[255, 0, 0]);
    page.close();
}
