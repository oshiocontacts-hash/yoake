//! # html-panel
//!
//! HTML のページを、頭なし（`--headless=new`）の Edge / Chrome で描き、絵（RGBA）とデータのやり取りを
//! mod に渡す。Horizon には依存しない（絵をテクスチャに上げるのは使う側）。
//!
//! ```no_run
//! use html_panel::{Options, Panel, Source};
//! use serde_json::json;
//!
//! // ブラウザの起動は裏のスレッドで行う。ここは待たない（Edge も Chrome も無い時だけ Err → 自前の画面に切り替える）
//! let panel = Panel::open_source(
//!     Source::File("mods/nestgate/ui/hub.html".into()), // ローカルの元はネットに出ない（Options::allow で許可を足せる）
//!     [640, 360],                                     // CSS ピクセル
//!     2.0,                                            // GUI の倍率（絵は 1280×720）
//!     Options { watch: cfg!(debug_assertions), ..Default::default() }, // 開発中はファイルが変わると読み直す
//! ).unwrap();
//! loop {
//!     panel.post(&json!({ "friends": 3 })).unwrap(); // ページの window.yoake.onMessage(obj) に届く
//!     for msg in panel.poll() { println!("{msg}"); } // ページの window.yoake.send(obj)
//!     let at = panel.to_page([12.0, 30.0], [320.0, 180.0]); // ウィジェットの中の座標 → ページの座標
//!     panel.click(at);
//!     if let Some(frame) = panel.frame() {               // 新しいコマがある時だけ（RGBA、幅×高さ×4）
//!         let _ = (frame.width, frame.height, frame.rgba);
//!     }
//!     # break;
//! }
//! panel.set_visible(false); // 画面に出ていない間は止める
//! // panel を捨てると（または close で）ブラウザを閉じる
//! ```
//!
//! ## ページ側
//!
//! 開いたページには、どのスクリプトより先に `window.yoake` が入る。
//!
//! - `window.yoake.send(value)`：mod の `Panel::poll` に届く（JSON にして送る。上限を超えると例外）。`post` は同じもの。
//! - `window.yoake.onMessage = (value) => { ... }`：mod の `Panel::post` の値。入れる前に届いた分は溜めておく。
//! - `window.yoake.onState = (state) => { ... }`：mod の `Panel::set_state`（`send`）の値。読み込み直しのあとに、
//!   最後の値がもう一度届く（状態として扱う時に）。
//!
//! ## 作り
//!
//! - 絵：`Page.startScreencast` の PNG（文字がにじまない）。変わった時だけ届く。倍率は開く時に決める
//!   （頭なしでは後から変えても細かさが変わらないので、GUI の倍率が変わったら開き直す）。
//! - 通信：`Options::allow` の外は `Fetch` で止める（http・https）。ローカルの元は既定でネットに出ない。
//! - 音：出さない（`--mute-audio`）。
//! - 起動：用意の命令の返事を全部待ってからページへ移る（待たないと、最初の文書に用意が間に合わないことがある）。
//! - 片付け：`close`／Drop でブラウザを終わらせる。Windows ではジョブ（KILL_ON_JOB_CLOSE）に入れるので、
//!   ゲームが落ちてもブラウザは残らない。使い捨てのプロファイル（一時フォルダ）も消す。

pub mod browser;
pub mod input;
pub mod keys;
pub mod proc;
pub mod win;
pub mod ws;

pub use input::Input;

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};

use crate::ws::{Message, WsWriter};

/// ページに、どのスクリプトより先に入れる橋渡し。
const BRIDGE: &str = r#"(() => {
  if (window.yoake) return;
  const LIMIT = __LIMIT__;
  let handler = null, onmsg = null;
  const queue = [], mq = [];
  const deliver = (s) => { try { handler(s); } catch (e) { console.error(e); } };
  const give = (m) => { try { onmsg(m); } catch (e) { console.error(e); } };
  const send = (v) => {
    const t = JSON.stringify(v === undefined ? null : v);
    if (t.length > LIMIT) throw new Error('yoake.send: message is ' + t.length + ' bytes, over ' + LIMIT);
    window.yoakePost(t);
  };
  window.yoake = {
    send,
    post: send,
    get onMessage() { return onmsg; },
    set onMessage(f) {
      onmsg = typeof f === 'function' ? f : null;
      while (onmsg && mq.length) give(mq.shift());
    },
    _message(m) {
      if (onmsg) { give(m); return; }
      if (mq.length >= 256) mq.shift();
      mq.push(m);
    },
    get onState() { return handler; },
    set onState(f) {
      handler = typeof f === 'function' ? f : null;
      while (handler && queue.length) deliver(queue.shift());
    },
    _deliver(s) {
      if (handler) { deliver(s); return; }
      if (queue.length >= 256) queue.shift();
      queue.push(s);
    },
  };
})();"#;

/// タブの音を録る（`Options::audio`）：getDisplayMedia でこのタブだけを、ローカルでは鳴らさずに録り、
/// 48 kHz・2 ch・16 bit を `htmlPanelAudio("A:<base64>")` で送る。うまくいったかは "S:ok" か "S:<理由>"。
/// 起動に `--auto-accept-this-tab-capture` が要る（選ぶ画面を出さない）。無音の間は送らない。
const TAB_AUDIO: &str = r#"(() => {
  if (window.__hpCap) return;
  window.__hpCap = 1;
  const post = (s) => { try { window.htmlPanelAudio(s); } catch (_) {} };
  (async () => {
    try {
      const s = await navigator.mediaDevices.getDisplayMedia({
        video: true,
        audio: { suppressLocalAudioPlayback: true, echoCancellation: false, noiseSuppression: false, autoGainControl: false, channelCount: 2, sampleRate: 48000 },
        preferCurrentTab: true, selfBrowserSurface: 'include', systemAudio: 'exclude', surfaceSwitching: 'exclude',
      });
      s.getVideoTracks().forEach((t) => t.stop());
      const tracks = s.getAudioTracks();
      if (!tracks.length) { post('S:no audio track'); return; }
      const ctx = new AudioContext({ sampleRate: 48000 });
      const src = ctx.createMediaStreamSource(new MediaStream(tracks));
      const sp = ctx.createScriptProcessor(2048, 2, 2);
      const mute = ctx.createGain();
      mute.gain.value = 0;
      src.connect(sp); sp.connect(mute); mute.connect(ctx.destination);
      sp.onaudioprocess = (e) => {
        const b = e.inputBuffer;
        const l = b.getChannelData(0), r = b.numberOfChannels > 1 ? b.getChannelData(1) : l;
        const out = new Int16Array(l.length * 2);
        let loud = false;
        for (let i = 0; i < l.length; i++) {
          const a = Math.max(-1, Math.min(1, l[i])), c = Math.max(-1, Math.min(1, r[i]));
          out[2 * i] = a * 32767; out[2 * i + 1] = c * 32767;
          if (a !== 0 || c !== 0) loud = true;
        }
        if (!loud) return;
        const u8 = new Uint8Array(out.buffer);
        let bin = '';
        for (let i = 0; i < u8.length; i += 0x8000) bin += String.fromCharCode.apply(null, u8.subarray(i, i + 0x8000));
        post('A:' + btoa(bin));
      };
      setInterval(() => { if (ctx.state !== 'running') ctx.resume(); }, 500);
      window.__hpKeep = [s, ctx, src, sp, mute];
      post('S:ok');
    } catch (e) {
      window.__hpCap = 0;
      post('S:' + e.name + ': ' + e.message);
    }
  })();
})()"#;

/// 溜めておくタブの音の上限（2 秒ぶん）。
const MAX_AUDIO: usize = 48_000 * 2 * 2;
/// 答えを待っている eval の上限（古い方から捨てる）。
const MAX_EVALS: usize = 64;
/// eval の番号札（パネルをまたいで重ならない）。
static EVAL_TICKETS: AtomicU64 = AtomicU64::new(1);

/// 溜めておくページからの便りの上限（読まれなければ古い方から捨てる）。
const MAX_POSTS: usize = 1024;
/// 1 通の上限（どちら向きも）の既定。
pub const DEFAULT_MAX_MESSAGE: usize = 1 << 20;
/// ローカルの元（File・Html）で、許可リストを決めなかった時に通す URL（ネットには出ない）。
pub const LOCAL_ONLY: [&str; 4] = ["file://", "data:", "blob:", "about:"];

/// 開く元。
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// mod の中のローカルの HTML ファイル（同じフォルダの CSS・JS・画像も読める）。
    File(PathBuf),
    /// HTML の文字列（一時フォルダに書いて開く）。
    Html(String),
    /// URL。
    Url(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelError {
    /// Chrome も Edge も見つからない（mod はふつうの画面に戻す）。
    NoBrowser,
}

impl fmt::Display for PanelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PanelError::NoBrowser => write!(f, "Chrome or Edge was not found (set HTML_PANEL_BROWSER to its exe)"),
        }
    }
}

impl std::error::Error for PanelError {}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// ブラウザを起動している最中（この間の send・input は、つながった時にまとめて送る）。
    Starting,
    /// ページがつながった。
    Ready,
    /// 起動に失敗した・ブラウザが落ちた（文字は理由）。mod はふつうの画面に戻す。
    Failed(String),
    /// close した。
    Closed,
}

/// 今のページの様子。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageInfo {
    pub url: String,
    pub title: String,
    pub loading: bool,
}

/// 1 コマ（RGBA、上の行から。`rgba.len() == width * height * 4`）。
#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// 通し番号（1 から）。
    pub seq: u64,
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    /// 使うブラウザ（無ければ `browser::find`）。
    pub browser: Option<PathBuf>,
    /// ブラウザのプロファイル（クッキーなどを残したい時）。無ければ一時フォルダを使い、閉じた時に消す。
    /// 同じフォルダを 2 つのパネルで同時には使えない。
    pub profile: Option<PathBuf>,
    /// 背景を透明にする（ページが背景を塗らなければ、透けた絵になる）。
    pub transparent: bool,
    /// 通してよい URL の頭（`https://example.com/` など）。それ以外の通信は止める。
    /// None なら、ローカルの元（File・Html）は `LOCAL_ONLY` だけ、URL の元は全部通す。
    pub allow: Option<Vec<String>>,
    /// 開発用：ローカルの元のフォルダを見て、ファイルが変わったら読み直す。
    pub watch: bool,
    /// 1 通の上限（バイト、どちら向きも）。0 なら `DEFAULT_MAX_MESSAGE`。
    pub max_message: usize,
    /// 窓ありで開く（画面の外に隠した窓。前面は取らない）。音を出す時に要る（頭なしは音を本当には鳴らさない）。
    /// 窓ありは Windows だけ。ほかの OS では頭なしで開く。
    pub window: bool,
    /// タブの音を録る（`audio_pcm` で取り出す。PC からは鳴らさない）。`window` の時だけ効く。
    pub audio: bool,
    /// 絵を JPEG（画質 1〜100）で受ける。`encoded_frame` で元の JPEG も取れる（配信用）。None なら PNG。
    pub jpeg: Option<u8>,
    /// ウェブのページだけ（埋め込む本体用）：file:// を読む許可を付けず、ダウンロードを止め、上の枠が
    /// http・https・data・blob・about:blank 以外へ移ったら about:blank に戻す。
    pub web_only: bool,
}

/// `web_only` の時に、上の枠が居てよい URL か。
pub fn web_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    ["https://", "http://", "data:", "blob:"].iter().any(|p| lower.starts_with(p)) || lower == "about:blank"
}

struct Inner {
    status: Status,
    writer: Option<WsWriter>,
    /// つながる前に頼まれた命令（つながったら送る）。
    queued: Vec<String>,
    frame: Option<Frame>,
    posts: VecDeque<Value>,
    /// 最後に send した値（読み込み直しのあとに、もう一度渡す）。
    state: Option<String>,
    size: [u32; 2],
    scale: f32,
    proc: Option<Arc<proc::Proc>>,
    /// 通してよい URL の頭（None は全部）。
    allow: Option<Vec<String>>,
    max_message: usize,
    info: PageInfo,
    /// 最後に届いたコマの元のデータ（JPEG か PNG）と通し番号。
    encoded: Option<(u64, Arc<Vec<u8>>)>,
    jpeg: Option<u8>,
    audio: VecDeque<i16>,
    audio_status: Option<Result<(), String>>,
    /// 答えを待っている命令：eval の番号札と、題を読む命令。
    pending: HashMap<u64, Pending>,
    evals: HashMap<u64, Option<Result<String, String>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Eval(u64),
    Title,
}

struct Shared {
    inner: Mutex<Inner>,
    open: AtomicBool,
    next_id: AtomicU64,
    frames: AtomicU64,
}

fn lock<V>(m: &Mutex<V>) -> MutexGuard<'_, V> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Shared {
    fn command(&self, method: &str, params: Value) -> String {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        json!({ "id": id, "method": method, "params": params }).to_string()
    }

    /// つながっていれば送る。まだなら溜める。
    fn cdp(&self, method: &str, params: Value) {
        let text = self.command(method, params);
        let mut inner = lock(&self.inner);
        let written = inner.writer.as_mut().map(|w| w.text(&text));
        match written {
            Some(Ok(())) => {}
            Some(Err(e)) => {
                inner.writer = None;
                if inner.status == Status::Ready {
                    inner.status = Status::Failed(format!("DevTools write: {e}"));
                }
            }
            None if inner.status == Status::Starting => inner.queued.push(text),
            None => {}
        }
    }

    fn fail(&self, why: String) {
        let mut inner = lock(&self.inner);
        if matches!(inner.status, Status::Starting | Status::Ready) {
            inner.status = Status::Failed(why);
        }
    }

    /// 命令を送り、答えを待つ印を付ける。
    fn cdp_pending(&self, method: &str, params: Value, pending: Pending) -> u64 {
        // 番号を先に取り、答えを待つ印を付けてから送る（ほかのスレッドの命令と番号が入れ替わらないように）
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let text = json!({ "id": id, "method": method, "params": params }).to_string();
        let mut inner = lock(&self.inner);
        inner.pending.insert(id, pending);
        let written = inner.writer.as_mut().map(|w| w.text(&text));
        match written {
            Some(Ok(())) => {}
            Some(Err(e)) => {
                inner.writer = None;
                if inner.status == Status::Ready {
                    inner.status = Status::Failed(format!("DevTools write: {e}"));
                }
            }
            None if inner.status == Status::Starting => inner.queued.push(text),
            None => {}
        }
        id
    }

    fn screencast_params(&self) -> Value {
        let i = lock(&self.inner);
        let (size, scale) = (i.size, i.scale);
        let mut v = json!({
            "format": "png",
            "maxWidth": (size[0] as f32 * scale).round() as u32,
            "maxHeight": (size[1] as f32 * scale).round() as u32,
            "everyNthFrame": 1,
        });
        if let Some(q) = i.jpeg {
            v["format"] = json!("jpeg");
            v["quality"] = json!(q.clamp(1, 100));
        }
        v
    }

    fn screencast(&self) {
        let params = self.screencast_params();
        self.cdp("Page.stopScreencast", json!({}));
        self.cdp("Page.startScreencast", params);
    }

    fn read_title(&self) {
        self.cdp_pending("Runtime.evaluate", json!({ "expression": "document.title", "returnByValue": true }), Pending::Title);
    }

    fn deliver_state(&self) {
        let state = lock(&self.inner).state.clone();
        if let Some(s) = state {
            self.cdp("Runtime.evaluate", json!({ "expression": format!("window.yoake && window.yoake._deliver({s})") }));
        }
    }
}

pub struct Panel {
    shared: Arc<Shared>,
}

static PANELS: AtomicU64 = AtomicU64::new(0);

impl Panel {
    /// `url` を幅 `width`×高さ `height`（CSS ピクセル）で開く。`scale` は描く細かさ（1.0〜4.0）。
    /// ブラウザの起動は裏のスレッドで行い、ここは待たない。Chrome も Edge も無い時だけ Err。
    pub fn open(url: &str, width: u32, height: u32, scale: f32) -> Result<Panel, PanelError> {
        Self::open_with(url, width, height, scale, Options::default())
    }

    pub fn open_with(url: &str, width: u32, height: u32, scale: f32, options: Options) -> Result<Panel, PanelError> {
        Self::open_source(Source::Url(url.to_owned()), [width, height], scale, options)
    }

    /// 元（ローカルの HTML か URL）を開く。`size` は CSS ピクセル、`scale` は描く細かさ（GUI の倍率を渡す）。
    /// 絵は `size × scale` ピクセルになる。倍率を変える時は開き直す。
    pub fn open_source(source: Source, size: [u32; 2], scale: f32, options: Options) -> Result<Panel, PanelError> {
        let (width, height) = (size[0], size[1]);
        let exe = options.browser.clone().filter(|p| p.is_file()).or_else(browser::find).ok_or(PanelError::NoBrowser)?;
        let size = [width.clamp(16, 4096), height.clamp(16, 4096)];
        let scale = if scale.is_finite() { scale.clamp(0.25, 4.0) } else { 1.0 };
        let shared = Arc::new(Shared {
            inner: Mutex::new(Inner {
                status: Status::Starting,
                writer: None,
                queued: Vec::new(),
                frame: None,
                posts: VecDeque::new(),
                state: None,
                size,
                scale,
                proc: None,
                allow: match (&options.allow, &source) {
                    (Some(list), _) => Some(list.clone()),
                    (None, Source::Url(_)) => None,
                    (None, _) => Some(LOCAL_ONLY.iter().map(|s| s.to_string()).collect()),
                },
                max_message: if options.max_message == 0 { DEFAULT_MAX_MESSAGE } else { options.max_message },
                info: PageInfo::default(),
                encoded: None,
                jpeg: options.jpeg,
                audio: VecDeque::new(),
                audio_status: None,
                pending: HashMap::new(),
                evals: HashMap::new(),
            }),
            open: AtomicBool::new(true),
            next_id: AtomicU64::new(1),
            frames: AtomicU64::new(0),
        });
        let n = PANELS.fetch_add(1, Ordering::Relaxed);
        let scratch = std::env::temp_dir().join("html-panel").join(format!("{}-{n}", std::process::id()));
        // 元を URL に（HTML の文字列は一時フォルダに書く）。見張るフォルダも決める
        let (url, watch_dir) = match &source {
            Source::Url(u) => (u.clone(), None),
            Source::File(p) => {
                let p = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
                (file_url(&p), p.parent().map(|d| d.to_path_buf()))
            }
            Source::Html(html) => {
                let dir = scratch.join("source");
                let _ = std::fs::create_dir_all(&dir);
                let file = dir.join("index.html");
                let _ = std::fs::write(&file, html);
                (file_url(&file), Some(dir))
            }
        };
        if options.watch {
            if let Some(dir) = watch_dir {
                watch(shared.clone(), dir);
            }
        }
        let s = shared.clone();
        let spawned = thread::Builder::new().name("html-panel".to_owned()).spawn(move || {
            let (profile, temporary) = match options.profile {
                Some(p) => (p, false),
                None => (scratch.join("profile"), true),
            };
            let mode = Launch {
                transparent: options.transparent,
                window: options.window && cfg!(windows),
                audio: options.audio && options.window && cfg!(windows),
                web_only: options.web_only,
            };
            if let Err(e) = run(&s, &exe, &profile, &url, mode) {
                s.fail(e);
            }
            // ここに来るのは、閉じた・落ちた・起動できなかった時
            let proc = lock(&s.inner).proc.take();
            drop(proc);
            if temporary {
                remove_profile(scratch_of(&profile));
            }
        });
        if let Err(e) = spawned {
            shared.fail(format!("could not start the panel thread: {e}"));
        }
        Ok(Panel { shared })
    }

    pub fn status(&self) -> Status {
        lock(&self.shared.inner).status.clone()
    }

    /// Rust → JS：ページの `window.yoake.onState(value)` に渡す。
    pub fn send(&self, value: &Value) {
        lock(&self.shared.inner).state = Some(value.to_string());
        self.shared.deliver_state();
    }

    /// Rust → JS：ページの `window.yoake.onMessage(value)` に 1 通送る（onMessage を入れる前の分は溜まる）。
    /// 上限（`Options::max_message`）を超えると Err。
    pub fn post(&self, value: &Value) -> Result<(), String> {
        let text = value.to_string();
        let max = lock(&self.shared.inner).max_message;
        if text.len() > max {
            return Err(format!("message is {} bytes, over {max}", text.len()));
        }
        self.shared.cdp("Runtime.evaluate", json!({ "expression": format!("window.yoake && window.yoake._message({text})") }));
        Ok(())
    }

    /// 状態を渡す（`send` と同じ。ページでは onState、読み込み直しのあとにもう一度届く）。
    pub fn set_state(&self, value: &Value) {
        self.send(value);
    }

    /// 読み直す。
    pub fn reload(&self) {
        self.shared.cdp("Page.reload", json!({ "ignoreCache": true }));
    }

    /// 画面に出ていない間は止める（描かず、スクリプトも止まる）。出たら戻す。
    pub fn set_visible(&self, visible: bool) {
        if visible {
            self.shared.cdp("Page.setWebLifecycleState", json!({ "state": "active" }));
            self.shared.screencast();
        } else {
            self.shared.cdp("Page.stopScreencast", json!({}));
            self.shared.cdp("Page.setWebLifecycleState", json!({ "state": "frozen" }));
        }
    }

    /// ウィジェットの中の座標（左上が 0、`widget` はウィジェットの大きさ、単位は何でもよい）をページの CSS ピクセルに。
    pub fn to_page(&self, at: [f32; 2], widget: [f32; 2]) -> [f64; 2] {
        let size = self.size().0;
        let f = |v: f32, w: f32, s: u32| if w > 0.0 { (v / w).clamp(0.0, 1.0) as f64 * s as f64 } else { 0.0 };
        [f(at[0], widget[0], size[0]), f(at[1], widget[1], size[1])]
    }

    /// マウスを動かす（`buttons` は押している最中のボタン：左 1・右 2・中 4。ドラッグは左を押したまま動かす）。
    pub fn mouse_move(&self, at: [f64; 2], buttons: u32) {
        self.input(&Input::Move { x: at[0], y: at[1], buttons });
    }

    /// ボタンを押す・離す（`button` は "left" / "right" / "middle"、`clicks` はダブルクリックなら 2）。
    pub fn mouse_button(&self, at: [f64; 2], button: &str, down: bool, clicks: u32) {
        let (x, y, button, clicks, mods) = (at[0], at[1], button.to_owned(), clicks.max(1), 0);
        self.input(&if down { Input::Down { x, y, button, clicks, mods } } else { Input::Up { x, y, button, clicks, mods } });
    }

    /// 左クリック（押して離す）。
    pub fn click(&self, at: [f64; 2]) {
        self.mouse_button(at, "left", true, 1);
        self.mouse_button(at, "left", false, 1);
    }

    /// ホイール（ピクセル。下へは dy が正）。
    pub fn wheel(&self, at: [f64; 2], dx: f64, dy: f64) {
        self.input(&Input::Wheel { x: at[0], y: at[1], dx, dy, mods: 0 });
    }

    /// 文字にならないキー（`key.keyboard.enter` などの名前、`mods` は DevTools の修飾）。
    pub fn key(&self, name: &str, down: bool, mods: u32) {
        self.input(&Input::Key { name: name.to_owned(), down, mods });
    }

    /// 文字を打つ（IME の確定文字もこれで入れる）。
    pub fn text(&self, text: &str) {
        self.input(&Input::Type(text.to_owned()));
    }

    /// 今のページの URL・題・読み込み中か。
    pub fn info(&self) -> PageInfo {
        lock(&self.shared.inner).info.clone()
    }

    /// 録ったタブの音（48 kHz・2 ch・16 bit、左右交互）を全部取り出す。`Options::audio` の時だけ溜まる。
    pub fn audio_pcm(&self) -> Vec<i16> {
        lock(&self.shared.inner).audio.drain(..).collect()
    }

    /// タブの音を録れているか（None はまだ分からない、Err は録れない理由）。ページが替わるたびに録り直す。
    pub fn audio_status(&self) -> Option<Result<(), String>> {
        lock(&self.shared.inner).audio_status.clone()
    }

    /// 最後に届いたコマの元のデータ（`Options::jpeg` なら JPEG）と通し番号。何度でも読める（配信用）。
    pub fn encoded_frame(&self) -> Option<(u64, Arc<Vec<u8>>)> {
        lock(&self.shared.inner).encoded.clone()
    }

    /// JPEG の画質（1〜100）を変える（変わった時だけ絵の受け方をやり直す）。PNG で開いたパネルは JPEG になる。
    pub fn set_jpeg_quality(&self, quality: u8) {
        let q = quality.clamp(1, 100);
        let changed = {
            let mut i = lock(&self.shared.inner);
            let changed = i.jpeg != Some(q);
            i.jpeg = Some(q);
            changed
        };
        if changed {
            self.shared.screencast();
        }
    }

    /// JavaScript を走らせ、答え（JSON の文字）を番号札で受け取る（`eval_result`）。
    pub fn eval_value(&self, expression: &str) -> u64 {
        let ticket = EVAL_TICKETS.fetch_add(1, Ordering::Relaxed);
        {
            let mut i = lock(&self.shared.inner);
            if i.evals.len() >= MAX_EVALS {
                if let Some(oldest) = i.evals.keys().min().copied() {
                    i.evals.remove(&oldest);
                }
            }
            i.evals.insert(ticket, None);
        }
        self.shared.cdp_pending("Runtime.evaluate", json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }), Pending::Eval(ticket));
        ticket
    }

    /// `eval_value` の答え。まだなら None。受け取った答えは手元から消える。
    pub fn eval_result(&self, ticket: u64) -> Option<Result<String, String>> {
        let mut i = lock(&self.shared.inner);
        match i.evals.get(&ticket) {
            Some(Some(_)) => i.evals.remove(&ticket).flatten(),
            Some(None) => None,
            None => Some(Err("unknown or expired eval ticket".to_owned())),
        }
    }

    /// JS → Rust：ページが `yoake.send` / `yoake.post` / `yoakePost` で送ったもの（届いた順）。
    pub fn poll(&self) -> Vec<Value> {
        lock(&self.shared.inner).posts.drain(..).collect()
    }

    /// クリック・ホイール・キーなど（座標は CSS ピクセル）。
    pub fn input(&self, input: &Input) {
        for (method, params) in input.to_cdp() {
            self.shared.cdp(method, params);
        }
    }

    /// 新しいコマがある時だけ返す（返したコマは手元から消える）。
    pub fn frame(&self) -> Option<Frame> {
        lock(&self.shared.inner).frame.take()
    }

    /// 今までに届いたコマの数。
    pub fn frames(&self) -> u64 {
        self.shared.frames.load(Ordering::Relaxed)
    }

    /// CSS の大きさ（幅, 高さ）と倍率。
    pub fn size(&self) -> ([u32; 2], f32) {
        let i = lock(&self.shared.inner);
        (i.size, i.scale)
    }

    /// 大きさ（CSS ピクセル）を変える。倍率は開いた時のまま（頭なしでは、あとから変えても絵の細かさが変わらない）。
    pub fn resize(&self, width: u32, height: u32) {
        lock(&self.shared.inner).size = [width.clamp(16, 4096), height.clamp(16, 4096)];
        let (size, scale) = self.size();
        self.shared.cdp("Emulation.setDeviceMetricsOverride", json!({ "width": size[0], "height": size[1], "deviceScaleFactor": scale, "mobile": false }));
        self.shared.screencast();
    }

    /// 戻る・進む。
    pub fn back(&self) {
        self.input(&Input::Back);
    }

    pub fn forward(&self) {
        self.input(&Input::Forward);
    }

    pub fn navigate(&self, url: &str) {
        self.input(&Input::Nav(url.to_owned()));
    }

    /// JavaScript を走らせる（答えは返さない。答えが欲しい時はページから yoake.post で送る）。
    pub fn eval(&self, expression: &str) {
        self.shared.cdp("Runtime.evaluate", json!({ "expression": expression }));
    }

    /// ブラウザを閉じる（Drop でも閉じる）。
    pub fn close(&self) {
        if !self.shared.open.swap(false, Ordering::Relaxed) {
            return;
        }
        let proc = {
            let mut i = lock(&self.shared.inner);
            i.status = Status::Closed;
            if let Some(mut w) = i.writer.take() {
                w.close();
            }
            i.queued.clear();
            i.proc.clone()
        };
        // 読む係はソケットが閉じて抜ける。プロセスはここで終わらせる（ジョブが子プロセスも片付ける）
        if let Some(p) = proc {
            p.kill();
        }
    }
}

impl Drop for Panel {
    fn drop(&mut self) {
        self.close();
    }
}

/// `C:\a\b.html` → `file:///C:/a/b.html`（空白などは % に）。
pub fn file_url(path: &std::path::Path) -> String {
    let text = path.display().to_string().replace('\\', "/");
    let text = text.strip_prefix("//?/").unwrap_or(&text).to_owned();
    let mut out = String::from("file://");
    if !text.starts_with('/') {
        out.push('/');
    }
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' | b':' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 一時プロファイルの入れ物（`.../<pid>-<n>/profile` の親）。
fn scratch_of(profile: &std::path::Path) -> PathBuf {
    profile.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| profile.to_path_buf())
}

/// 開発用：フォルダの中のファイル（2 段下まで、2000 個まで）の一番新しい更新時刻。
fn newest(dir: &std::path::Path, depth: u32, seen: &mut usize) -> Option<std::time::SystemTime> {
    let mut best = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        *seen += 1;
        if *seen > 2000 {
            break;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let t = if meta.is_dir() {
            if depth == 0 {
                continue;
            }
            newest(&entry.path(), depth - 1, seen)
        } else {
            meta.modified().ok()
        };
        best = best.max(t);
    }
    best
}

/// 開発用：フォルダのファイルが変わったら読み直す（0.5 秒ごとに見る）。
fn watch(shared: Arc<Shared>, dir: PathBuf) {
    let _ = thread::Builder::new().name("html-panel-watch".to_owned()).spawn(move || {
        let mut last = newest(&dir, 2, &mut 0);
        while shared.open.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(500));
            let now = newest(&dir, 2, &mut 0);
            if now != last {
                last = now;
                shared.cdp("Page.reload", json!({ "ignoreCache": true }));
            }
        }
    });
}

fn remove_profile(profile: PathBuf) {
    // ブラウザが抜けきるまで、ファイルが掴まれていることがある
    for _ in 0..25 {
        if std::fs::remove_dir_all(&profile).is_ok() || !profile.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(200));
    }
}

/// JPEG を RGBA に戻す。
pub fn decode_jpeg(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let options = zune_jpeg::zune_core::options::DecoderOptions::default().jpeg_set_out_colorspace(zune_jpeg::zune_core::colorspace::ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(bytes, options);
    let rgba = decoder.decode().map_err(|e| format!("{e:?}"))?;
    let (w, h) = decoder.dimensions().ok_or("jpeg without a size")?;
    if rgba.len() != w * h * 4 {
        return Err(format!("jpeg decoded to {} bytes for {w}x{h}", rgba.len()));
    }
    Ok((w as u32, h as u32, rgba))
}

/// PNG を RGBA に戻す。
pub fn decode_png(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("png too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    buf.truncate(info.buffer_size());
    let (w, h) = (info.width, info.height);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("indexed png was not expanded".to_owned()),
    };
    if rgba.len() != w as usize * h as usize * 4 {
        return Err(format!("png decoded to {} bytes for {w}x{h}", rgba.len()));
    }
    Ok((w, h, rgba))
}

/// 起動のしかた。
#[derive(Clone, Copy, Debug)]
struct Launch {
    transparent: bool,
    /// 窓あり（隠した窓）。Windows だけ。
    window: bool,
    /// タブの音を録る（窓ありの時だけ）。
    audio: bool,
    web_only: bool,
}

fn run(s: &Arc<Shared>, exe: &std::path::Path, profile: &std::path::Path, url: &str, mode: Launch) -> Result<(), String> {
    let transparent = mode.transparent;
    std::fs::create_dir_all(profile).map_err(|e| format!("profile folder {}: {e}", profile.display()))?;
    let _ = std::fs::remove_file(profile.join("DevToolsActivePort"));
    let (size, scale, max_message, limited) = {
        let i = lock(&s.inner);
        (i.size, i.scale, i.max_message, i.allow.is_some())
    };
    let mut args: Vec<String> = vec![
        "--remote-debugging-port=0".into(),
        "--remote-allow-origins=*".into(),
        format!("--user-data-dir={}", profile.display()),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--hide-scrollbars".into(),
        "--disable-features=Translate,MediaRouter,OptimizationHints,GlobalMediaControls".into(),
        "--disable-background-timer-throttling".into(),
        "--disable-renderer-backgrounding".into(),
        "--disable-backgrounding-occluded-windows".into(),
        format!("--window-size={},{}", size[0], size[1]),
        // 頭なしでは、Emulation の deviceScaleFactor だけだと screencast が CSS ピクセルのまま届く
        format!("--force-device-scale-factor={scale}"),
    ];
    if !mode.web_only {
        // file:// のページから、同じフォルダの JSON や画像を fetch できるように
        args.push("--allow-file-access-from-files".into());
    }
    if mode.window {
        // 窓あり：最初の窓は作らない（最初の窓は前に出てしまう）。窓は Target.createTarget で、前に出さずに作る
        args.push(format!("--window-position={},{}", win::HIDDEN_X, win::HIDDEN_Y));
        args.push("--no-startup-window".into());
        args.push("--autoplay-policy=no-user-gesture-required".into());
        args.push("--disable-features=CalculateNativeWinOcclusion".into());
    } else {
        args.insert(0, "--headless=new".into());
        args.push("about:blank".into());
    }
    if mode.audio {
        // タブの音を録る時、選ぶ画面を出さずに受け入れる（TAB_AUDIO）
        args.push("--auto-accept-this-tab-capture".into());
    } else {
        args.push("--mute-audio".into());
    }
    // 調べ用：HTML_PANEL_LOG=<ファイル> でブラウザのログを書く
    if let Some(log) = std::env::var_os("HTML_PANEL_LOG") {
        args.insert(0, "--enable-logging".into());
        args.insert(1, format!("--log-file={}", std::path::Path::new(&log).display()));
    }
    let proc = Arc::new(if mode.window { win::spawn(exe, &args)? } else { proc::spawn(exe, &args, None)? });
    if mode.window {
        // 窓の見張り：出てきた窓をすぐ画面の外へ（最初の 15 秒は細かく）
        let (proc, shared) = (proc.clone(), s.clone());
        let _ = thread::Builder::new().name("html-panel-windows".to_owned()).spawn(move || {
            let started = std::time::Instant::now();
            while shared.open.load(Ordering::Relaxed) && proc.alive() {
                let size = lock(&shared.inner).size;
                win::hide_windows(proc.pid, size);
                thread::sleep(Duration::from_millis(if started.elapsed() < Duration::from_secs(15) { 30 } else { 500 }));
            }
        });
    }
    {
        let mut i = lock(&s.inner);
        if !s.open.load(Ordering::Relaxed) {
            return Ok(());
        }
        i.proc = Some(proc.clone());
    }
    let (port, browser_path) = browser::wait_port(profile, Duration::from_secs(20), || proc.alive() && s.open.load(Ordering::Relaxed))?;
    if !s.open.load(Ordering::Relaxed) {
        return Ok(());
    }

    // 頭なし：起動時に開いたページ（about:blank）を使う。無ければ作る。窓あり：前に出さない新しい窓を作る
    let existing = if mode.window {
        None
    } else {
        let targets = browser::call(port, &browser_path, "Target.getTargets", json!({}))?;
        targets
            .get("targetInfos")
            .and_then(Value::as_array)
            .and_then(|list| list.iter().find(|t| t.get("type").and_then(Value::as_str) == Some("page")))
            .and_then(|t| t.get("targetId"))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let create = if mode.window {
        json!({ "url": "about:blank", "newWindow": true, "background": true, "left": win::HIDDEN_X, "top": win::HIDDEN_Y, "width": size[0] + 16, "height": size[1] + 120 })
    } else {
        json!({ "url": "about:blank" })
    };
    let target = match existing {
        Some(id) => id,
        None => browser::call(port, &browser_path, "Target.createTarget", create)?
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or("DevTools gave no target id")?
            .to_owned(),
    };
    let (mut reader, mut writer) = ws::connect(port, &format!("/devtools/page/{target}")).map_err(|e| e.to_string())?;

    // 用意の命令を先に、そのあと待っていた命令を送ってから、つながったことにする
    let mut setup = vec![
        s.command("Page.enable", json!({})),
        s.command("Runtime.enable", json!({})),
        s.command("Runtime.addBinding", json!({ "name": "yoakePost" })),
        s.command("Page.addScriptToEvaluateOnNewDocument", json!({ "source": BRIDGE.replace("__LIMIT__", &max_message.to_string()) })),
        s.command("Emulation.setDeviceMetricsOverride", json!({ "width": size[0], "height": size[1], "deviceScaleFactor": scale, "mobile": false })),
        s.command("Emulation.setFocusEmulationEnabled", json!({ "enabled": true })),
    ];
    if mode.audio {
        setup.push(s.command("Runtime.addBinding", json!({ "name": "htmlPanelAudio" })));
    }
    if mode.web_only {
        // ページから PC にファイルを落とさせない
        setup.push(s.command("Page.setDownloadBehavior", json!({ "behavior": "deny" })));
    }
    if limited {
        // 許可リストの外の通信を止める（Fetch.requestPaused で一つずつ決める）
        // ローカルのファイル（file:・data:・blob:）はネットに出ないので、掛けない
        setup.push(s.command("Fetch.enable", json!({ "patterns": [{ "urlPattern": "http://*" }, { "urlPattern": "https://*" }] })));
    }
    if transparent {
        setup.push(s.command("Emulation.setDefaultBackgroundColorOverride", json!({ "color": { "r": 0, "g": 0, "b": 0, "a": 0 } })));
    }
    setup.push(s.command("Page.startScreencast", s.screencast_params()));
    // 用意の命令の返事を全部待ってから、ページへ移る。起動直後の最初のページでは、返事を待たずに移ると
    // 用意（橋渡しのスクリプトなど）が最初の文書に間に合わないことがあった。2 秒で返事がそろわなければ送り直す
    let ids = |list: &[String]| -> HashSet<u64> {
        list.iter().filter_map(|t| serde_json::from_str::<Value>(t).ok()?.get("id")?.as_u64()).collect()
    };
    for attempt in 0..3 {
        for text in &setup {
            writer.text(text).map_err(|e| format!("DevTools write: {e}"))?;
        }
        let mut waiting = ids(&setup);
        let started = std::time::Instant::now();
        // 返事が来なくても進めるよう、この間だけ読みに期限を付ける
        let _ = reader.set_timeout(Some(Duration::from_millis(300)));
        while !waiting.is_empty() && started.elapsed() < Duration::from_secs(2) {
            match reader.read() {
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
                Ok(Message::Text(t)) => {
                    if let Some(id) = serde_json::from_str::<Value>(&t).ok().and_then(|v| v.get("id").and_then(Value::as_u64)) {
                        waiting.remove(&id);
                    }
                }
                Ok(Message::Ping(p)) => {
                    let _ = writer.pong(&p);
                }
                Ok(Message::Close) | Err(_) => return Err("DevTools closed while setting up".to_owned()),
            }
        }
        let _ = reader.set_timeout(None);
        if waiting.is_empty() {
            break;
        }
        eprintln!("html-panel: setup was not answered ({} left), sending it again ({}/3)", waiting.len(), attempt + 1);
        setup = setup.iter().filter_map(|t| serde_json::from_str::<Value>(t).ok()).map(|v| s.command(v["method"].as_str().unwrap_or(""), v["params"].clone())).collect();
    }
    let navigate = s.command("Page.navigate", json!({ "url": url }));
    {
        let mut i = lock(&s.inner);
        if !s.open.load(Ordering::Relaxed) {
            writer.close();
            return Ok(());
        }
        for text in std::iter::once(&navigate).chain(i.queued.iter()) {
            writer.text(text).map_err(|e| format!("DevTools write: {e}"))?;
        }
        i.queued.clear();
        i.writer = Some(writer);
        i.status = Status::Ready;
        i.info.url = url.to_owned();
        i.info.loading = true;
    }

    let mut crashes = 0u32;
    loop {
        let message = match reader.read() {
            Ok(m) => m,
            Err(e) => {
                if s.open.load(Ordering::Relaxed) {
                    let why = if proc.alive() { format!("DevTools connection lost: {e}") } else { "the browser quit".to_owned() };
                    s.fail(why);
                }
                return Ok(());
            }
        };
        let text = match message {
            Message::Text(t) => t,
            Message::Ping(p) => {
                if let Some(w) = lock(&s.inner).writer.as_mut() {
                    let _ = w.pong(&p);
                }
                continue;
            }
            Message::Close => {
                if s.open.load(Ordering::Relaxed) {
                    s.fail("DevTools closed the connection".to_owned());
                }
                return Ok(());
            }
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
        // 答え：eval の番号札と、題
        if let Some(id) = value.get("id").and_then(Value::as_u64) {
            let mut i = lock(&s.inner);
            match i.pending.remove(&id) {
                Some(Pending::Eval(ticket)) => {
                    let answer = if let Some(e) = value.get("error") {
                        Err(e.to_string())
                    } else if let Some(e) = value.pointer("/result/exceptionDetails") {
                        Err(e.pointer("/exception/description").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| e.to_string()))
                    } else {
                        Ok(value.pointer("/result/result/value").map(|v| v.to_string()).unwrap_or_else(|| "null".to_owned()))
                    };
                    if let Some(slot) = i.evals.get_mut(&ticket) {
                        *slot = Some(answer);
                    }
                }
                Some(Pending::Title) => {
                    if let Some(t) = value.pointer("/result/result/value").and_then(Value::as_str) {
                        i.info.title = t.chars().take(200).collect();
                    }
                }
                None => {}
            }
            continue;
        }
        let Some(method) = value.get("method").and_then(Value::as_str) else { continue };
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        match method {
            "Page.screencastFrame" => {
                if let Some(session) = params.get("sessionId").and_then(Value::as_u64) {
                    s.cdp("Page.screencastFrameAck", json!({ "sessionId": session }));
                }
                let Some(data) = params.get("data").and_then(Value::as_str) else { continue };
                // 展開はロックの外で。PNG は変わった時だけ届くので、ここで展開しても追いつく
                let bytes = match base64::engine::general_purpose::STANDARD.decode(data) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("html-panel: frame: {e}");
                        continue;
                    }
                };
                let decoded = if bytes.starts_with(&[0xFF, 0xD8]) { decode_jpeg(&bytes) } else { decode_png(&bytes) };
                match decoded {
                    Ok((width, height, rgba)) => {
                        let seq = s.frames.fetch_add(1, Ordering::Relaxed) + 1;
                        let mut i = lock(&s.inner);
                        i.frame = Some(Frame { width, height, rgba, seq });
                        i.encoded = Some((seq, Arc::new(bytes)));
                    }
                    Err(e) => eprintln!("html-panel: frame: {e}"),
                }
            }
            "Runtime.bindingCalled" if params.get("name").and_then(Value::as_str) == Some("htmlPanelAudio") => {
                let payload = params.get("payload").and_then(Value::as_str).unwrap_or("");
                if let Some(b64) = payload.strip_prefix("A:") {
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                        let mut i = lock(&s.inner);
                        i.audio.extend(bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])));
                        let extra = i.audio.len().saturating_sub(MAX_AUDIO);
                        i.audio.drain(..extra - extra % 2);
                    }
                } else if let Some(status) = payload.strip_prefix("S:") {
                    lock(&s.inner).audio_status = Some(if status == "ok" { Ok(()) } else { Err(status.chars().take(200).collect()) });
                }
            }
            "Runtime.bindingCalled" => {
                if params.get("name").and_then(Value::as_str) != Some("yoakePost") {
                    continue;
                }
                let payload = params.get("payload").and_then(Value::as_str).unwrap_or("");
                if payload.len() > lock(&s.inner).max_message {
                    eprintln!("html-panel: dropped a {} byte message from the page", payload.len());
                    continue;
                }
                let v = serde_json::from_str::<Value>(payload).unwrap_or_else(|_| Value::String(payload.to_owned()));
                let mut i = lock(&s.inner);
                if i.posts.len() >= MAX_POSTS {
                    i.posts.pop_front();
                }
                i.posts.push_back(v);
            }
            "Page.frameNavigated" => {
                let top = params.get("frame").map(|f| f.get("parentId").is_none()).unwrap_or(false);
                if top {
                    if let Some(u) = params.pointer("/frame/url").and_then(Value::as_str) {
                        if mode.web_only && !web_url(u) {
                            // file:・chrome: などへは居させない
                            s.cdp("Page.navigate", json!({ "url": "about:blank" }));
                            continue;
                        }
                        let mut i = lock(&s.inner);
                        i.info.url = u.to_owned();
                        i.audio_status = None;
                    }
                    // 別のサイトへ移ると描く係（プロセス）が替わって配信が止まるので、始め直す
                    s.screencast();
                }
            }
            // ページのスクリプトが読み終わったところで、最後の状態を渡し直す（onState が後から入っても溜まる）。
            // タブの音を録るなら、ここで始める（新しいページでは録り直し。ユーザーの操作の扱いで呼ぶ）
            "Page.domContentEventFired" => {
                s.deliver_state();
                s.read_title();
                if mode.audio {
                    s.cdp("Runtime.evaluate", json!({ "expression": TAB_AUDIO, "userGesture": true }));
                }
            }
            "Page.navigatedWithinDocument" => {
                if let Some(u) = params.get("url").and_then(Value::as_str) {
                    lock(&s.inner).info.url = u.to_owned();
                }
                s.read_title();
            }
            "Page.frameStartedLoading" => lock(&s.inner).info.loading = true,
            "Page.frameStoppedLoading" | "Page.loadEventFired" => {
                lock(&s.inner).info.loading = false;
                s.read_title();
            }
            "Fetch.requestPaused" => {
                let Some(id) = params.get("requestId").and_then(Value::as_str) else { continue };
                let url = params.pointer("/request/url").and_then(Value::as_str).unwrap_or("");
                let ok = lock(&s.inner).allow.as_ref().map(|list| list.iter().any(|p| url.starts_with(p.as_str()))).unwrap_or(true);
                if ok {
                    s.cdp("Fetch.continueRequest", json!({ "requestId": id }));
                } else {
                    s.cdp("Fetch.failRequest", json!({ "requestId": id, "errorReason": "BlockedByClient" }));
                }
            }
            // 描く係が落ちた：たまに起きるので、3 回までは開き直す
            "Inspector.targetCrashed" if crashes < 3 => {
                crashes += 1;
                eprintln!("html-panel: the page crashed; opening it again ({crashes}/3)");
                s.cdp("Page.navigate", json!({ "url": url }));
                s.screencast();
            }
            "Inspector.detached" | "Inspector.targetCrashed" => {
                s.fail(format!("{method}"));
                return Ok(());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_png() {
        // 2x1 の RGB（赤・緑）
        let mut out = Vec::new();
        {
            let mut e = png::Encoder::new(&mut out, 2, 1);
            e.set_color(png::ColorType::Rgb);
            e.set_depth(png::BitDepth::Eight);
            let mut w = e.write_header().unwrap();
            w.write_image_data(&[255, 0, 0, 0, 255, 0]).unwrap();
        }
        let (w, h, rgba) = decode_png(&out).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(rgba, vec![255, 0, 0, 255, 0, 255, 0, 255]);
    }
}
