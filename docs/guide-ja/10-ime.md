# 10. MOD 画面で IME（日本語入力）を使う

`client-full` の MOD は、自分の MOD 画面（`Screen::Mod`）で IME の変換途中の文字列（preedit）と確定文字列を受け取れます。OS の候補ウィンドウをテキストボックスのそばに置くサービスもあります。

対象は **サンドボックス化された WebAssembly MOD** です。Yoake 2.0 の full-access core モジュールでは `hooks::IME_PREEDIT` / `hooks::IME_COMMIT` と `stubs::f_services` を使います（[MOD-GUIDE.ja.md](../yoake2/MOD-GUIDE.ja.md) を参照）。

## 10-1. 購読と初期化

次の 4 つのフックを使います。

- `hook::SCREEN_INIT_POST`: 画面が開いたことを覚えておきます（サービスはここでは呼べません）。
- `hook::FRAME`: 次のフレームで IME を有効化し、候補ウィンドウの位置を決めます。
- `hook::IME_PREEDIT`: 変換途中の文字列を受け取ります。
- `hook::IME_COMMIT`: 確定した文字列を受け取ります。

> [!IMPORTANT]
> クライアントのサービスは、書き込み権限のあるフックからしか呼べません。`screen_init_post` のホストアクセスは読み取り専用なので、そこで呼ぶと `Err` になります。`frame` フックで呼び、成功するまで毎フレーム再試行してください。

```rust
use yoake_wasm_sdk::full::client::{ClientMod, et, ime_set_allowed, ime_set_cursor_area};
use yoake_wasm_sdk::full::hook;

#[derive(Default)]
struct TextScreen {
    composition: String,
    pending: bool,
}

impl ClientMod for TextScreen {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::SCREEN_INIT_POST, hook::FRAME, hook::IME_PREEDIT, hook::IME_COMMIT]
    }

    fn screen_init_post(&mut self, _screen: et::NativeScreenView) {
        // 画面を開くたびに IME の状態はリセットされるので、また設定する。
        self.pending = true;
    }

    fn frame(&mut self, _elapsed: u64) {
        if self.pending
            && ime_set_allowed(true).is_ok()
            && ime_set_cursor_area(32.0, 48.0, 180.0, 20.0).is_ok()
        {
            self.pending = false;
        }
    }

    fn ime_preedit(&mut self, text: String, cursor: Option<(u32, u32)>) {
        self.composition = text;
        println!("cursor: {cursor:?}");
    }

    fn ime_commit(&mut self, text: String) {
        self.composition.clear();
        println!("committed: {text}");
    }
}

yoake_wasm_sdk::export_client_full!(TextScreen);
```

完全なコンパイル例は `crates/yoake-wasm-sdk/examples/ime/` にあります。実際の画面では、既存の MOD 画面 API で自分の画面を開き、`screen_render_post` で `composition` をテキストボックス内に描画します。

IME の有効状態と候補位置は MOD 画面を開くとき・閉じるときにリセットされます。テキストボックスからフォーカスが外れたときは `ime_set_allowed(false)` を呼びます。もう一度フォーカスしたときに `true` を設定してください。

**既存の MOD への影響はありません。** `ime_set_allowed` も `ime_set_cursor_area` も一度も呼ばない MOD 画面は、このサービスが追加される前とまったく同じに動きます（タイトル画面から開いた MOD 画面では IME が有効、ゲーム内の MOD 画面では無効）。どちらかを呼んだ時点から、その画面は MOD の指定に従います。

## 10-2. フックの動作

### `ime_preedit(text, cursor)`

- `text` は現在の変換途中文字列です。空文字列は変換状態のクリアを表します。OS から `Ime::Disabled` が来たときも、空文字列と `None` で通知されます。
- `cursor` は `Some((start, end))` のとき、**UTF-8 バイト列のオフセット**で選択範囲を表します。文字数や UTF-16 のインデックスではありません。範囲がない場合は `None` です。
- エンジンは MOD 画面の preedit を自動描画しません。MOD が自分のテキストボックスに描きます。

### `ime_commit(text)`

IME が確定した文字列を、既存の `screen_char_typed_pre` より先に 1 回だけ受け取ります。そのあと、従来どおり文字ごとの `screen_char_typed_pre` も発生します。

> [!IMPORTANT]
> `ime_commit` で文字列を挿入するなら、同じ MOD 画面の `screen_char_typed_pre` でも同じ文字を挿入しないでください。二重入力になります。既存の文字フックだけを使う実装も、そのまま動きます。

これらのフックが発生するのは、**MOD 画面が開いていて、かつ MOD が購読している場合だけ**です。バニラのチャットや設定画面の IME 内容は配信されません。購読していない MOD は呼び出されません。

## 10-3. サービスと座標

### `ime_set_allowed(allowed)`

自分の MOD 画面で OS の IME を許可するかどうかを設定します。呼ぶまでは従来の動作のままです（上記）。変更は次の IME 同期でウィンドウへ反映されます。`false` を設定すると、その画面の preedit と候補位置もクリアされます。

### `ime_set_cursor_area(x, y, w, h)`

> [!WARNING]
> **必ず `yoake_wasm_sdk::full::client::ime_set_cursor_area(x, y, w, h)`（ラッパー）を使ってください。** 生成された生の WIT インポート `services::ime_set_cursor_area` は、引数がアルファベット順に並べ替えられていて **`(h, w, x, y)`** の順です。生のほうに `(x, y, w, h)` を渡すと値が入れ替わり、候補ウィンドウが違う場所に出ます。ラッパーが順序を直します。

候補ウィンドウの基準となる矩形を設定します。通常はテキストボックスまたはキャレットの領域を渡します。

- `x`, `y`: 画面左上からの位置
- `w`, `h`: 幅と高さ（0 以上）
- 単位: 他の画面 API と同じ **GUI スケーリング済み座標**で、物理ウィンドウピクセルではありません

エンジンが現在の GUI スケールで物理ピクセルへ変換します。幅と高さは少なくとも 1 物理ピクセルになります。NaN・無限大・負の幅または高さはエラーです。同じ矩形を毎フレーム設定しても、ウィンドウへの再設定は変更があったときだけ行われます。

どちらのサービスも、**呼び出した MOD 自身の画面が開いていること**が必要です。他の MOD の画面が開いているときや、MOD 画面以外から呼ぶと `Err` を返します。`full::client::ime_set_allowed` と `full::client::ime_set_cursor_area` は SDK の薄いラッパーです（`ime_set_allowed` は生の `services::` と同じ）。

`service.ime` はカタログ上の機能識別子で、サービス ID は `ime_set_allowed` が **201**、`ime_set_cursor_area` が **202** です。`service.storage` や `service.gpu` のような `[wasm] permissions` の宣言は不要で、`permissions = ["service.ime"]` は書かないでください。これらの ID は full-access のフック ID・エンジン関数 ID とは別の名前空間です。
