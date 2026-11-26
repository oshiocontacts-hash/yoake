# 15. 埋め込みブラウザとメディア表示（MEDIA1）

`client-full` ワールドの MOD は、PC にインストールされている **Edge または Chrome** を裏側で開き、その画面をテクスチャ（`hud::GuiTexture`）として HUD やワールド上に描画できます。入力や JavaScript の実行、タブ音声の PCM 取得にも対応しています。

> [!NOTE]
> この機能は `client-full` のクライアントモジュール専用です。`yak.toml` の `[wasm]` に `permissions = ["service.browser"]` を宣言する必要があります（宣言していないとエラーになります）。

---

## 15-1. `yak.toml` の設定

`crates/yoake-wasm-sdk/examples/browser-demo/yak.toml` の抜粋です。

```toml
[side]
client = true
server = false
required_on_join = false

[wasm]
interface = "1.7"
world = "client-full"
client = "bin/wasm/browser-demo.wasm"
permissions = ["service.browser"]
```

---

## 15-2. `Browser` の使い方（`browser-demo`）

[`crates/yoake-wasm-sdk/examples/browser-demo/src/lib.rs`](../../crates/yoake-wasm-sdk/examples/browser-demo/src/lib.rs) は、`B` キーで埋め込みブラウザを切り替え、最新フレームを `hud::GuiTexture` にコピーして画面左上へ表示します。

```rust
use yoake_wasm_sdk::full::client::bindings::yoake::mods::types::{TextureFilter, TextureFormat};
use yoake_wasm_sdk::full::client::media::{Browser, BrowserOptions};
use yoake_wasm_sdk::full::client::{self, et, hud, ClientMod};
use yoake_wasm_sdk::full::hook;

#[derive(Default)]
struct BrowserDemo {
    view: Option<(Browser, hud::GuiTexture, String)>,
}

impl ClientMod for BrowserDemo {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::HUD, hook::KEY]
    }

    fn key(&mut self, key: String, pressed: bool) {
        if !pressed || !key.eq_ignore_ascii_case("b") {
            return;
        }
        if self.view.is_some() {
            self.view = None; // Drop で自動的にブラウザが閉じます
            return;
        }
        let options = BrowserOptions {
            allow: Vec::new(),
            audio: false,
            jpeg_quality: 80,
            max_message: 64 * 1024,
            transparent: false,
            window: false,
        };
        let Ok(browser) = Browser::open("https://example.com", 640, 360, 1.0, &options) else {
            return;
        };
        let Ok(tex) =
            hud::GuiTexture::create(640, 360, TextureFormat::Rgba8, TextureFilter::Linear)
        else {
            return;
        };
        if let Ok(id) = client::gui_texture_id(&tex) {
            self.view = Some((browser, tex, id));
        }
    }

    fn hud(&mut self) -> et::HudOutcome {
        let mut batch_commands = Vec::new();
        if let Some((browser, tex, id)) = &self.view {
            let _ = browser.frame_to_texture(tex);
            batch_commands.push(et::HudCommand::Texture(et::HudTexture {
                id: id.clone(),
                x: 16.0,
                y: 16.0,
                width: 320.0,
                height: 180.0,
                color: 0xffff_ffff,
            }));
        }
        et::HudOutcome {
            batch_commands,
            corners_commands: Vec::new(),
        }
    }
}

yoake_wasm_sdk::export_client_full!(BrowserDemo);
```

ワールド内に描きたいときは、`ext::draw_world_textured_quad` や `ext::draw_world_billboard` に同じ `GuiTexture` を渡します。

---

## 15-3. 主なメソッド（`yoake_wasm_sdk::full::client::media`）

| メソッド / 関数 | できること |
|---|---|
| `Browser::open(url, width, height, scale, &options)` | ブラウザを開きます（`Drop` で自動終了）。 |
| `browser.frame_to_texture(&tex)` | 最新フレームを `GuiTexture` に RGBA8 で書き込みます（更新がなければ `Ok(None)`）。 |
| `browser.frame_jpeg(quality)` / `media::jpeg_to_texture(&bytes, &tex)` | 他のプレイヤーへ送る JPEG 取得と、受信側でのデコード（最大 4 MiB・3840×2160）。 |
| `browser.audio_pcm()` | タブの音声（48 kHz・2ch `Vec<i16>`）を取得します。`window: true` かつ `audio: true` で開いたときのみ有効で、`ext::AudioStream::push_i16` でゲーム内に流せます。 |
| `browser.navigate(url)` / `back()` / `forward()` / `reload()` / `set_visible(bool)` | ページ移動・履歴・非表示時の一時停止。 |
| `browser.input(&event)` | マウス・ホイール・キー入力・テキスト・ペーストを送ります。 |
| `browser.eval(js)` / `Browser::eval_result(ticket)` | ページ内で JS を実行し、JSON の結果を受け取ります（式 64 KiB まで、結果 1 MiB まで）。 |
| `browser.post(json)` / `browser.messages()` | ページ側とメッセージを送受信します。 |

---

## 15-4. 安全上の制限と仕様

- **ウェブページ専用**: `http://`、`https://`、`data:`、`blob:`、`about:blank` だけを開けます。`file://`、`chrome://`、`edge://`、`javascript:` などは拒否され、ファイルのダウンロードも無効化されます。
- **パッケージ専用プロファイル**: `<game>/media-browser/<mod id>/<slot>` に保存され、別 MOD や普段使っているブラウザとは分離されます。
- **上限**: 1 パッケージあたり同時に **8 個**、解像度は最大 **3840×2160**、URL は **8 KiB**、`allow` プレフィックスは **64 個**、メッセージは **1 MiB** までです。
- **終了処理**: MOD をアンロード／リロードすると、その MOD が開いたブラウザはすべて閉じられます。
- `"service.screen-share"` は今後の画面共有機能用に予約された権限名で、現時点では未接続です。
