# FR-MEDIA1: service.browser for client-full packages (media-services)

Branch `yoake-free/media-browser`, from `yoake-free/merge` 43b66a190. FR-MERGE4 (fed161c18) was merged in
before the last regeneration. Requested for WebScreen's move to a `.yak`
(`animagic/mods/webscreen/WASM-NEEDS.md`). Nothing launches the game. When no package declares
`service.browser`, nothing changes: no browser starts, no thread starts, and no file is written.

## What a package gets

A new additive interface pair, `media-types` and `media-services`, is imported by the `client-full`
world. The services are in `abi-subset-media.json` (catalog 1.0.0, ids **240–254**). The client catalog
moves 1.5.0 → **1.6.0**. `client-services` did not grow. `abi_1_0_is_additive` passes: only a new
interface, new functions and new permission names were added.

| id | service | what it does |
|---|---|---|
| 240 | `browser-open(url, width, height, scale, options) -> u32` | Opens a page (CSS size × scale pixels). `options`: `audio`, `window`, `transparent`, `allow` (http/https prefixes), `jpeg-quality` (0 = PNG frames), `max-message`. |
| 241 | `browser-close(handle)` | Closes it (the process ends). |
| 242 | `browser-set-visible(handle, bool)` | Stops drawing and freezes the page while hidden. |
| 243 | `browser-navigate(handle, url)` | Goes to a new page. |
| 244 | `browser-history(action, handle)` | Goes back, forward, or reloads. |
| 245 | `browser-input(event, handle)` | Mouse move, down, up and wheel; key down and up (vanilla key names); text; insert (paste or IME). |
| 246 | `browser-state(handle) -> page-state` | Status, error, url, title, loading, frames, frame width and height, audio status. |
| 247 | `browser-frame-to-texture(handle, borrow<gui-texture>) -> option<u64>` | Writes the newest frame into the package's own texture (RGBA8, top-left, clipped to the texture). Returns the frame number, or none if that texture already has it. |
| 248 | `browser-frame-jpeg(handle, quality) -> option<(u64, list<u8>)>` | The newest frame as JPEG, for streaming it to other players. Each frame is returned once. |
| 249 | `jpeg-to-texture(bytes, borrow<gui-texture>) -> (w, h)` | Decodes a received JPEG on the host (at most 4 MiB and 3840×2160) and writes it into a texture. Decoding a 1280×720 frame inside WASM came close to the 20 ms limit. |
| 250 | `browser-audio-pcm(handle) -> list<s16>` | The tab's sound: 48 kHz, 2 channels, interleaved. It is not played on the PC. Only for a browser opened with `window` and `audio`. |
| 251 | `browser-eval(expression, handle) -> u64` | Runs JavaScript in that page and returns a ticket. Expressions are at most 64 KiB; at most 256 tickets may be waiting. |
| 252 | `browser-eval-result(ticket) -> eval-answer` | `{done, ok, value}`, where `value` is JSON. Each answer is handed out once, and answers are at most 1 MiB. |
| 253 | `browser-post(handle, json)` | Sends a message to the page (html-panel's `onMessage`). |
| 254 | `browser-messages(handle) -> list<string>` | Messages the page sent with `yoake.post` / `yoake.send`, in order. |
| 255–259 | reserved | Screen share (phase 2, below). |

Sound and frames go through the existing paths. The package pushes PCM into its own `audio-stream-*`
(positional, Records volume) and draws its textures with `draw-world-textured-quad` or the HUD. The
host does not add a second audio or drawing path.

SDK: `yoake_wasm_sdk::full::client::{browser, media_types}`.

## Permissions

The manifest permissions `"service.browser"` and `"service.screen-share"` are now in `WASM_PERMISSIONS`.
Both are refused unless the package has a `wasm.client` with `world = "client-full"`
(`horizon-mods` manifest check, test `media_permissions_need_a_client_full_module`). The guide's
permission list (04) and its error text (05) are updated in both languages.

* A package without `service.browser` gets `"service.browser permission is required"` from every call
  (yoake-wasm `MediaServices`, `ClientHost::media_services` is `None`). The grant is set when the package
  loads, from its manifest (`wasm_mods.rs`, `media_services::grant`).
* **The install permission check** happens in the launcher, which this repository does not contain. The
  launcher reads `[wasm] permissions` from the archive's manifest. It should name `service.browser`
  ("opens web pages in a hidden Edge/Chrome; can play and capture their sound") and `service.screen-share`
  when it asks. The join offer (`join_mods::request_json`) does not carry permissions. I did not change
  that wire format; it would need a protocol change.

## The browser and its limits (horizon-client `wasm_mods/media_services.rs`)

* **Only installed Edge or Chrome** is started, through `html-panel` (`browser::find`: Edge first). No
  other program is started.
  * A package cannot pass a browser path.
  * The `HTML_PANEL_BROWSER` override is a user environment variable, as before.
  * There are no other process launches.
* **Profile per package:** `<game>/media-browser/<mod id>/<slot>`, with slots 0–7. Two browsers of one
  package never share a profile, and packages never share one. The folder is outside the package's
  data folder.
* **Eval only in the package's own pages:** tickets are bound to the handle. Closing a browser drops its
  tickets, and a package can only name its own handles (handles are per `GameHost`).
* **Web pages only** (`Options::web_only`, new in html-panel):
  * `browser-open`, `browser-navigate` and `allow` accept only http, https, data and blob URLs, plus
    `about:blank`.
  * `--allow-file-access-from-files` is not passed.
  * Downloads are denied (`Page.setDownloadBehavior deny`).
  * A top frame that still reaches another scheme is sent back to `about:blank`.
  * Tested: `file:///C:/Users/`, `chrome://`, `edge://`, `javascript:`, `view-source:` and `about:config` are refused.
* **Limits:**
  * 8 browsers per package.
  * 3840×2160 drawn pixels.
  * URLs up to 8 KiB.
  * 64 allow prefixes.
  * Messages up to html-panel's 1 MiB.
* **Lazy:** a package's `Browsers` are made at its first `browser-open`. html-panel starts its threads
  (reader, and the window watcher in window mode) and the browser process only then.
* **Unload:** `release_resources` closes every browser of the package. The process tree ends (job
  object), and `Drop` closes them too.
* **Host time:** frame copies and JPEG decoding count as host time (`add_host_time`), as the other client
  services do.

### html-panel in the engine

`crates/html-panel` is the same crate as `animagic/mods/html-panel`: it has no Horizon dependency and its
own lints. It now has:

* `window` (a hidden off-screen window that never takes focus or the foreground; Windows only)
* `audio` (tab capture with `getDisplayMedia` and `preferCurrentTab`, sent as PCM through a binding; the
  PC stays silent)
* `jpeg` (JPEG screencast frames, plus the encoded bytes)
* `eval_value` / `eval_result`
* `info` (url, title, loading)
* `back` / `forward`
* `web_only`

Setup handshake: navigation waits for every setup answer. This fixed the crashed first page and the
missing bridge (20 of 20 runs on Edge and Chrome).

## Tests

| What | Result |
|---|---|
| `cargo test -p horizon-client --lib media_services` | 12 pass, no browser (pages are test doubles behind `set_opener`): idle start, permission gate, profile and `web_only`, refused schemes, limits, input/history/visibility/state, audio only with a window, JPEG once per frame, eval tickets, post/messages, unload closes all, clipping |
| `… real_browser_draws_a_page -- --ignored` | Passes with the installed Edge (headless): a red `data:` page gives a 64×48 frame |
| `cargo test -p html-panel` | 4 unit tests plus the doctest pass |
| `cargo test -p horizon-mods --features yak --test yak` | 43 pass, including the new manifest test |
| `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen --lib` | 46 pass, including `abi_1_0_is_additive` (after the MERGE4 merge as well) |
| `cargo check -p yoake-wasm-sdk --features full --target wasm32-wasip2` | Builds |
| `crates/yoake-bindgen/regen.sh` | exit 0, no changes (see the last section) |

Not tested:

* No guest `.wasm` that calls these services; the host trait and the permission gate are tested directly.
* `browser-frame-to-texture` and `jpeg-to-texture` writing into a real GPU texture. That needs a wgpu
  device. The clipping is tested, and the write uses the same `gui_texture_write` path as service
  `gui-texture-write`.

## Screen share: phase 2 (ids 255–259 reserved)

Not in this branch; only the permission name `service.screen-share` exists. The plan:

* Use the Windows `GraphicsCapturePicker`, the system's own picker. The package never sees window titles
  or the list of windows, and the user chooses each time.
* Services: `screen-share-pick() -> handle`, `-frame-to-texture`, `-frame-jpeg`, `-stop`.
* No per-application audio at first (a later item).

## Glue regeneration

`crates/yoake-bindgen/regen.sh` was run after the MERGE4 merge (exit 0). It changed no file's content
(glue, `ids.lock.json`, stubs and bindgen docs are byte-for-byte what MERGE4 committed, apart from line
endings). This is as expected: the media services are WIT services of the sandboxed `client-full` world,
not engine-call glue. `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen --lib` after the merge
regenerated `yoake-full.wit` unchanged (46 pass).
