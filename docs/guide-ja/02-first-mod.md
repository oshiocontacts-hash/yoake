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
yoake new <minigame|hud-only|custom-mob|utility|script|client-full|server-full|worker|full-access> <name>
```

### 選べる 9 種類のテンプレート

CLI の仕様と生成物は `crates/yoake-cli/src/scaffold.rs` と `main.rs` に基づきます。[^cli] `script` だけプレースホルダーで、残りの 8 つはそのまま `yoake build` と `yoake check` が通る実用的なひな形です。

| テンプレート名 | WIT ワールド | 生成されるひな形 |
|---|---|---|
| `hud-only` | `client-mod` | 画面（HUD）に文字を表示するクライアント MOD |
| `minigame` | `client-mod` | クライアント側のミニゲーム開始点（サーバーのゲーム処理には別のサーバー MOD が必要） |
| `custom-mob` | `server-full` | F4M カスタム Mob（`content/` のモデル・アニメ・定義 + `ScriptGoal` AI、`permissions = ["service.mobs"]`） |
| `utility` | `client-mod` | キー入力をログに出すクライアント MOD |
| `script` | `client-mod` | スクリプト連携は未実装のプレースホルダー |
| `client-full` | `client-full` | フルカタログ（`ClientMod`）のクライアント MOD |
| `server-full` | `server-full` | フルカタログ（`ServerMod`）のサーバー MOD |
| `worker` | `server-full` + `worker-mod` | 通信・重いファイル処理用の WASI p2 ワーカーをペアにしたサーバー MOD |
| `full-access` | `full-access` | `yoake-guest-sdk` を使う `#![no_std]` フルアクセス・コア Wasm MOD |

> [!IMPORTANT]
> `<name>` は **3〜48 文字**で、**英小文字で始まり、英小文字・数字・単一のハイフンだけ**を使えます。アンダースコア（`_`）、末尾のハイフン、連続したハイフン（`--`）は `yoake new` の `<name>` では使えません（`yak.toml` の `id` そのものは `_` も許します。[5 章の ⑦](./05-errors.md) を参照）。Windows の予約名（`con`、`prn`、`aux`、`nul`、`com1`〜`com9`、`lpt1`〜`lpt9`）も拒否されます。
>
> `<name>` はそのまま `yak.toml` の **`id`**（ただし `custom-mob` は名前空間にするため `-` を `_` に変換）、Rust の**クレート名**、**`bin/wasm/<name>.wasm`** のファイル名になります。

> [!NOTE]
> **WIT ワールド（MOD の種類）**は、MOD が使えるフックとサービスの組み合わせです。セーブデータのワールドとは別の意味です。
>
> - **`client-mod`**: 19 個の基本フックで足りる場合に選びます。`world` を省略した場合の既定で、生 `.wasm` のホットリロードにも対応します。
> - **`client-full`**: アイテムアイコン、ツールチップ、FOV、GPU、スプライト差し替え、ブラウザなどの機能が必要な場合に選びます。`.yak` としてインストールして使います。
> - **`server-full`**: `server = "..."` を指定したサーバーモジュールの既定です（`world = "server-full"` は書かず、`server` パスだけで選ばれます）。
> - **`worker-mod`**: `[wasm] worker` に指定する、通信や重いファイル I/O 専用の別スレッド・コンポーネントです（[12 章](./12-files-network.md)）。
> - **`inject-mod`**: エンジン関数に割り込む Mixin 用です。`interface = "1.1"` または `"1.2"` が必要で、クライアント／サーバーの両方に書けます（[8 章](./08-mixin.md)）。
> - **`full-access`**: エンジン関数を直接呼ぶコア Wasm モジュール用です（[MOD-GUIDE.ja.md](../yoake2/MOD-GUIDE.ja.md)）。`yoake build`・`check`・`pack` はこれらすべての種類に対応しています。

---

## 2-2. 作られるファイルの説明

`yoake new hud-only my-hud` は、次のファイルとフォルダを作ります。

1. **`yak.toml`** — パッケージ ID（`yak.toml` の `id`）、バージョン、名前、作者、動作サイド、組み込む `.wasm` のパスを書くマニフェストです。
2. **`Cargo.toml`** — Rust のビルド設定です。SDK のパスと出力形式を指定します。
3. **`src/lib.rs`** — MOD の本体と、ローカルのモックを使うテストです。
4. **`README.md`** — ビルド、検査、パッケージ化、開発用インストールの説明です。
5. **`.gitignore`** — `target/`、`dist/`、`.yoake/` を Git の管理対象から外します。
6. **`.yoake/`** — SDK と WIT 定義のコピーです。`YOAKE_SDK_PATH` を指定して作成した場合は、コピーの代わりにその SDK を参照します。

### `yoake new hud-only my-hud` が実際に生成する 3 ファイル

#### `yak.toml`

```toml
format = 1
id = "my-hud"
version = "0.1.0"
name = "my-hud"
authors = ["Replace with your name"]
description = "A client-side HUD starter."

[yoake]
version = "*"

[side]
client = true
server = false
required_on_join = false

[wasm]
interface = "1.7"
client = "bin/wasm/my-hud.wasm"
```

`world` がないため、WIT ワールドは `client-mod` です（`1.0` 〜 `1.7` が指定できます）。

#### `Cargo.toml`

```toml
[package]
name = "my-hud"
version = "0.1.0"
edition = "2024"
publish = false

[lib]
crate-type = ["cdylib", "rlib"]

[dependencies]
yoake-wasm-sdk = { path = ".yoake/yoake-wasm-sdk" }

[features]
# `yoake test` builds the test module with this feature (it exports the tests instead of the mod).
tests = ["yoake-wasm-sdk/test"]

[profile.release]
opt-level = "s"
lto = true
strip = true
panic = "abort"

[workspace]
exclude = [".yoake/yoake-wasm-sdk", ".yoake/yoake-guest-sdk"]
```

`crate-type = ["cdylib", "rlib"]` により、WebAssembly コンポーネントと Rust ライブラリの両方をビルドできます。`[features]` の `tests` は、`yoake test` がテストモジュールをビルドするときに使うフィーチャです（[6. MOD をゲームエンジンの中でテストする（`06-testing.md`）](./06-testing.md) を参照）。

#### `src/lib.rs`

```rust
use yoake_wasm_sdk::{hook, hud, Mod};

#[derive(Default)]
struct Starter;

impl Mod for Starter {
    fn subscriptions() -> &'static [&'static str] { &[hook::HUD] }

    fn hud(&mut self) {
        let _ = hud::text("my-hud", (10.0, 10.0), 0xffffff, true);
    }
}

use yoake_wasm_sdk::testing::{BlockPos, TestCase, TestResult, TestWorld};

#[cfg(all(target_arch = "wasm32", not(feature = "tests")))]
yoake_wasm_sdk::export_mod!(Starter);

#[cfg(all(target_arch = "wasm32", feature = "tests"))]
yoake_wasm_sdk::export_tests!(YOAKE_TESTS);

#[yoake_wasm_sdk::yoake_test]
fn example_world_test(world: &mut TestWorld<'_>) -> TestResult {
    let position = BlockPos { x: 0, y: 64, z: 0 };
    world.place_block(position, "minecraft:stone")?;
    world.run_ticks(1)?;
    world.assert_block(position, "minecraft:stone")
}

pub const YOAKE_TESTS: &[TestCase] = &[TestCase { name: "example_world_test", run: example_world_test }];

#[cfg(test)]
mod tests {
    use super::*;
    use yoake_wasm_sdk::testing::TestDriver;

    #[derive(Default)]
    struct MockWorld { block: Option<(BlockPos, String)>, ticks: u32 }

    impl TestDriver for MockWorld {
        fn place_block(&mut self, position: BlockPos, block: &str) -> TestResult {
            self.block = Some((position, block.to_owned()));
            Ok(())
        }
        fn block_at(&mut self, position: BlockPos) -> Result<String, String> {
            self.block.as_ref().filter(|(at, _)| *at == position).map(|(_, block)| block.clone()).ok_or_else(|| "missing block".to_owned())
        }
        fn spawn_entity(&mut self, _kind: &str, _position: BlockPos) -> Result<u64, String> { Ok(1) }
        fn run_ticks(&mut self, ticks: u32) -> TestResult { self.ticks += ticks; Ok(()) }
    }

    #[test]
    fn example_world_scenario() {
        let mut world = MockWorld::default();
        YOAKE_TESTS[0].run(&mut world).unwrap();
        assert_eq!(world.ticks, 1);
    }
}
```

- **`subscriptions`**: 呼んでほしいフックを並べます。この例では `hook::HUD` だけを購読します。
- **`hud` メソッド**: 画面左上から `(10.0, 10.0)` に、白色・影ありの `my-hud` を描画します。
- **`export_mod!`**: WebAssembly 向けビルドでは、MOD をゲーム本体から呼び出せる形で公開します。
- **`export_tests!`**: `tests` フィーチャでビルドしたときだけ、MOD の代わりにテストモジュールを公開します。
- **テスト**: `cargo test` は実際のゲームではなく、ファイル内のモックを使って動きます。ゲームエンジンの中で動かすのは `yoake test` です（[6 章](./06-testing.md)）。ひな形の `y: 64` は空中で、スーパーフラットの地面のすぐ上は `y: -60` です。

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

[^cli]: 確認元: `crates/yoake-cli/src/scaffold.rs`（名前の検証、テンプレート、生成ファイル）と `crates/yoake-cli/src/main.rs`（ビルド、検査、パッケージ化）。
