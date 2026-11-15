# 2. 最初の MOD を作る

環境準備ができたら、さっそく最初の MOD を作ってみましょう。
ここでは画面（HUD）に文字を表示するシンプルなクライアント MOD（`my-hud`）を例に、プロジェクトの作成から `.yak` パッケージの出力までを一通り体験します。

---

## 2-1. `yoake new` でひな形を作る

作業フォルダでターミナル（PowerShell など）を開き、`yoake new` コマンドを実行します。

```sh
yoake new hud-only my-hud
```

コマンドの書式は次の通りです。

```sh
yoake new <minigame|hud-only|custom-mob|utility|script> <name>
```

### 選べる 5 種類のテンプレート

用途に合わせて次の 5 つのテンプレート名を指定できます（各テンプレートが生成する初期コードの細部は、このブランチのドキュメントには記載がないため **（未確定）** です）。

| テンプレート名 | 用途のイメージ |
|---|---|
| `hud-only` | 画面（HUD）への表示を行うクライアント向け MOD |
| `minigame` | ミニゲーム作成向けテンプレート（詳細未確定） |
| `custom-mob` | カスタムモブ作成向けテンプレート（詳細未確定） |
| `utility` | 便利機能・ユーティリティ向けテンプレート（詳細未確定） |
| `script` | スクリプト的な小規模 MOD 向けテンプレート（詳細未確定） |

> [!IMPORTANT]
> `<name>` に Windows で予約されている名前（`con`、`prn`、`aux`、`nul`、`com1`…、`lpt1`…）を指定すると拒否されます。なお、`yak.toml` の `id` の規則は次の通りです（`manifest.rs`）：**3〜48 文字の英小文字（`a-z`）・数字（`0-9`）・アンダースコア（`_`）・ハイフン（`-`）**（`<name>` がそのまま `id` になるかどうかは **（未確定）** です）。

---

## 2-2. 作られるファイルの説明

プロジェクトフォルダの中には、主に次の 4 つのファイルが作られます。

1. **`yak.toml`** — MOD の身分証明書（マニフェスト）です。MOD の ID、バージョン、名前、作者、動作サイド（クライアント／サーバー）、組み込む `.wasm` ファイルのパスなどを書きます。
2. **`Cargo.toml`** — Rust のビルド設定ファイルです。使用するクレート（`yoake-wasm-sdk`）や、出力形式（`crate-type = ["cdylib"]`）などを指定します。
3. **`src/lib.rs`** — MOD の本体となる Rust プログラムです。どのタイミング（フック）で何をするかをここに書きます。
4. **`.gitignore`** — Git で管理しない一時ファイル（ビルド途中の `target/` など）を指定するファイルです。

### 各ファイルの具体例

このブランチのドキュメント（`docs/yoake-wasm-dev.md`）およびサンプル（`crates/yoake-wasm-sdk/examples/`）にある実際のファイルを見てみましょう。

#### `yak.toml` の例（`examples/hud-item/yak.toml` より）

```toml
format = 1
id = "hud-item"
version = "0.1.0"
name = "HUD Item Demo"
authors = ["Yoake SDK Examples"]
description = "Client-side WebAssembly example that draws an item icon and label on the HUD."
license = "MIT"

[yoake]
version = ">=0.1"

[side]
client = true
server = false
required_on_join = false

[wasm]
interface = "1.0"
client = "bin/wasm/hud-item.wasm"
world = "client-full"
```

19 個の基本フックを使う標準の `client-mod` ワールドの場合は、`world` を省略して次のように書けます（`docs/yoake-wasm-dev.md` より）。

```toml
[wasm]
client = "bin/wasm/hello.wasm"
interface = "1.0"
```

#### `Cargo.toml` の例（`examples/hello/Cargo.toml` より）

```toml
[package]
name = "yoake-wasm-hello"
version = "0.1.0"
edition = "2024"
license = "MIT"
publish = false

[lib]
crate-type = ["cdylib"]

[dependencies]
yoake-wasm-sdk = { path = "../..", default-features = false, features = ["runtime"] }

[profile.release]
opt-level = "s"
lto = true
strip = true
panic = "abort"

[workspace]
```

- `crate-type = ["cdylib"]` によって `.wasm` コンポーネントが出力されます。
- `yoake-wasm-sdk` はデフォルトで Rust 標準ライブラリ（`std`）が使えます（`println!` や `HashMap` などが利用可能）。最小サイズの `.wasm` にしたい場合は上の例のように `default-features = false, features = ["runtime"]` を指定して `#![no_std]` で書くこともできます。
- **補足**: 上の `Cargo.toml` はリポジトリ内のサンプル（`examples/hello/Cargo.toml`）のものであり、`name = "yoake-wasm-hello"` や `path = "../.."` はリポジトリ内の位置に合わせた設定です。`yoake new` が実際に生成する `Cargo.toml` の内容（SDK の指定方法やパスなど）はこれとは異なる可能性があります **（未確定）**。

#### `src/lib.rs` の例（`docs/yoake-wasm-dev.md` より）

```rust
#![no_std]
use yoake_wasm_sdk::prelude::*;
use yoake_wasm_sdk::{Mod, export_mod, hook, hud};

#[derive(Default)]
struct Hello;

impl Mod for Hello {
    fn subscriptions() -> &'static [&'static str] { &[hook::HUD] }
    fn hud(&mut self) { let _ = hud::text("hi", (4.0, 4.0), 0xffffff, true); }
}
export_mod!(Hello);
```

- **ポイント 1（`subscriptions`）**: ゲーム本体に呼んでほしいフックの名前を `subscriptions()` に並べます。ここに書いたフック（上の例では `hook::HUD`）だけが毎フレーム呼ばれます。
- **ポイント 2（`hud` メソッド）**: `hud::text("hi", (4.0, 4.0), 0xffffff, true)` で、画面左上から座標 `(4.0, 4.0)` の位置に白色（`0xffffff`）・影あり（`true`）で `"hi"` と描画しています。
- **ポイント 3（`export_mod!`）**: 最後に `export_mod!(Hello);` と書くことで、ゲーム本体から呼び出せる形で公開されます。

---

## 2-3. ビルド・チェック・パッケージ化（`build` → `check` → `pack`）

コードが書けたら、`yoake` コマンドでビルドからパッケージ作成まで行います。

### ① `yoake build`（ビルドと検査）

```sh
yoake build
```

Rust のコードを WebAssembly（`.wasm`）にビルドし、続けてパッケージの検査（チェック）まで自動で行います。

### ② `yoake check`（検査のみ）

```sh
yoake check
```

ビルドは行わず、`yak.toml` の内容や `.wasm` ファイルが正しいかどうかだけを検査します。もしソースコード（`src/lib.rs` など）を書き換えたあとにビルドし忘れて `.wasm` が古いままになっている場合は、`stale`（古い）というエラーを出します。

### ③ `yoake pack`（`.yak` パッケージの作成）

```sh
yoake pack
```

ビルドと検査を行ったうえで、配布・導入用の `.yak` ファイルを出力します。

- **出力される `.yak` の場所**: プロジェクト内の **`dist/<id>-<version>.yak`** に作られます（たとえば `id = "my-hud"`、`version = "0.1.0"` なら `dist/my-hud-0.1.0.yak`）。実行するたびに毎回上書きされます。

---

## 2-4. コマンドの出力（JSON 1 行）の見方

`yoake` の各コマンドは、実行結果を標準出力（stdout）に **1 行の JSON** として表示します。たとえば次のような出力です。

```json
{"errors":[],"id":"demo-hud","modules":[{"path":"bin/wasm/demo-hud.wasm","side":"client","world":"client-mod"}],"ok":true,"version":"0.1.0","wasm_needs":[]}
```

- `"ok":true` かつ `"errors":[]` になっていれば成功です！
- もし設定ミスやフック名の間違いがある場合は、`"ok":false` になり、`"errors"` の中に原因のメッセージが入ります（詳しくは [5. よくあるエラーと直し方（`05-errors.md`）](./05-errors.md) を見てください）。

---

## 次のステップ

`.yak` ファイルや `.wasm` ファイルができたら、次は [3. ゲームに入れて遊ぶ・すぐ試す（`03-play.md`）](./03-play.md) に進んで、実際のゲーム画面で動かしてみましょう。
