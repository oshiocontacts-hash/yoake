# 6. MOD をゲームエンジンの中でテストする

前章までで、WebAssembly（Wasm）MOD を作成してビルド・パッケージ化する流れを学びました。この章では、**ワールド操作のシナリオ（と、サーバー側 MOD への `mod_call`）を実際のゲームエンジン（`horizon-client`）の中で自動テストする方法**を解説します。なお、テストモジュールはサーバー側のワールドだけを操作する別コンポーネントとして動くため、**クライアント専用の MOD（HUD など）の描画や画面は確かめられません**（2 クライアントのテストでは、2 つ目のクライアントの状態を `ClientView` で確かめられます。[6-5](#6-5-2-クライアントのテスト) を見てください）。

Yoakeのテスト機能（`yoake test`）を使うと、ウィンドウを表示しないオフスクリーン状態（`--offscreen`）でゲームが起動し、テストごとに毎回まっさらなスーパーフラットワールドを作ってテスト関数を実行してくれます。また、ゲームを起動せずに手元のモック（模擬ドライバ）だけでワールド操作のシナリオを素早く確かめることもできます。

---

## 6-1. テストを書く準備

### テストモジュールの仕組み

Yoakeのテストは、通常の MOD 本体（`client-mod` などの WIT ワールド）とは別の WebAssembly コンポーネントである**テストモジュール（test module）**としてビルドされます（WIT ワールドは `crates/yoake-wasm/wit/yoake-test.wit` の `test-mod`）。

- 通常のビルドでは MOD 本体（`bin/wasm/<id>.wasm`）が生成されます。
- Cargoフィーチャ `tests` を有効にしてビルドすると、同じクレートからテストモジュール（`bin/wasm/<id>-tests.wasm`）が生成されます。
- テストモジュールは `yak.toml` には記述せず、配布用の `.yak` パッケージにも含まれません。そのため、テスト用のコードが誤って配布用の MOD に混ざる心配がありません。

### `Cargo.toml` の設定（`tests` フィーチャ）

`yoake new` で作成したプロジェクトには最初から設定されていますが、手動で追加する場合は `Cargo.toml` に `tests = ["yoake-wasm-sdk/test"]` を定義します。[^scaffold]

```toml
[features]
tests = ["yoake-wasm-sdk/test"]
```

これにより、テストビルド時のみSDKの `test` フィーチャ（`yoake_wasm_sdk::testing::engine` と `export_tests!` マクロ）が有効になります。

### `#[yoake_test]` と `export_tests!` でテストを登録する

テスト関数は `&mut TestWorld<'_>` を受け取り `TestResult`（`Result<(), String>`）を返す関数として書きます。`Ok(())` を返せば成功、`Err(メッセージ)` を返せば失敗です（`?` 演算子でドライバのエラーをそのまま返せます）。

以下は構成例です（`Starter` は自分の MOD の型名に置き換えます）。[^wasm-dev]

```rust
use yoake_wasm_sdk::testing::{BlockPos, TestCase, TestResult, TestWorld};

fn places_a_block(world: &mut TestWorld<'_>) -> TestResult {
    let at = BlockPos { x: 0, y: -60, z: 0 };
    world.place_block(at, "minecraft:stone")?;
    world.run_ticks(2)?;                       // lets the server run 2 ticks
    world.assert_block(at, "minecraft:stone")
}

pub const YOAKE_TESTS: &[TestCase] = &[TestCase { name: "places_a_block", run: places_a_block }];

#[cfg(all(target_arch = "wasm32", not(feature = "tests")))]
yoake_wasm_sdk::export_mod!(Starter);                // the mod
#[cfg(all(target_arch = "wasm32", feature = "tests"))]
yoake_wasm_sdk::export_tests!(YOAKE_TESTS);          // the test module
```

`yoake new` のテンプレートのように、テスト関数には `#[yoake_wasm_sdk::yoake_test]`（または `use yoake_wasm_sdk::yoake_test;` して `#[yoake_test]`）属性を付けることができます。

```rust
#[yoake_wasm_sdk::yoake_test]
fn example_world_test(world: &mut TestWorld<'_>) -> TestResult {
    let position = BlockPos { x: 0, y: 64, z: 0 };
    world.place_block(position, "minecraft:stone")?;
    world.run_ticks(1)?;
    world.assert_block(position, "minecraft:stone")
}

pub const YOAKE_TESTS: &[TestCase] = &[TestCase { name: "example_world_test", run: example_world_test }];
```

> [!NOTE]
> `yoake new` のひな形では `y: 64`（空中の座標）が使われていますが、地面のすぐ上に置く場合は上の例のように `y: -60` 付近を使います。

- **`#[yoake_test]`**: テスト関数であることを示すマーカー属性です（引数は取らず、引数を渡すと `"yoake_test does not accept arguments"` でコンパイルエラーになります）。この属性単体では自動登録されないため、必ず `YOAKE_TESTS` スライス（`&[TestCase]`）にも登録してください。
- **`export_tests!(YOAKE_TESTS)`**: `feature = "tests"` のときに `YOAKE_TESTS` をテストモジュールの `tests` / `run-test` エクスポートとして公開します。
- **テスト名のルール**: 1〜128文字、モジュール内で重複なし、最大1024個までです。またコマンドライン引数として渡されるため、先頭を `-` にすることはできません。

---

## 6-2. `TestWorld` と `TestDriver` の全関数リファレンス

テスト関数が受け取る `TestWorld` は、裏側の `TestDriver` トレイト（実ゲームでは `engine::HostDriver`、ローカルの `cargo test` では自作モック）へ処理を渡します。すべての操作は統合サーバー（Integrated Server）スレッドのティック間で実行されます。

### `TestWorld` の公開メソッド一覧[^testing-rs]

| 関数名 | 動作 | 使用例 |
| --- | --- | --- |
| `TestWorld::new(driver)` | `TestDriver` の可変参照から新しい `TestWorld` を作る | `let mut world = TestWorld::new(&mut driver);` |
| `place_block(position, block)` | オーバーワールドの指定座標にブロックを設置する（`/setblock x y z <block>`） | `world.place_block(ORIGIN, "minecraft:stone")?;` |
| `block_at(position)` | 指定座標のブロックID文字列を取得する（プロパティなしの `namespace:id`） | `let id = world.block_at(ORIGIN)?;` |
| `spawn_entity(kind, position)` | 指定座標にエンティティを召喚し、そのエンティティID（`u64`）を返す | `let pig = world.spawn_entity("minecraft:pig", ORIGIN)?;` |
| `run_ticks(ticks)` | サーバーを `ticks` ティック（1〜72000）待機なしで高速に進める | `world.run_ticks(3)?;` |
| `break_block(position)` | プレイヤーが壊したようにドロップ付きでブロックを破壊する（`/setblock ... destroy`） | `world.break_block(ORIGIN)?;` |
| `spawn_mannequin(name, position)` | `name` から決まる固定UUIDのマネキン（非プレイヤー）を召喚しIDを返す | `let player = world.spawn_mannequin("Tester", ORIGIN)?;` |
| `run_command(command)` | サーバーコンソールとしてコマンドを実行し、成否（`bool`）を返す | `let ok = world.run_command("say hi")?;` |
| `assert_command(command)` | `run_command` を実行し、失敗（`false`）ならテストを失敗させる | `world.assert_command("say hi")?;` |
| `mod_call(target, method, payload)` | 読み込み済みのサーバーモジュール（テスト対象 MOD 自身、または `--yoake-test-with` で追加した MOD）の `handle-call` を呼び出す | `let echoed = world.mod_call("demo", "echo", b"ping")?;` |
| `entities_near(position, radius)` | `position` から半径 `radius` ブロック以内のエンティティ種別一覧（最大512件）を返す | `let kinds = world.entities_near(ORIGIN, 4.0)?;` |
| `player_names()` | サーバー上のプレイヤー名一覧（ワールドのオーナーが先頭）を返す | `let names = world.player_names()?;` |
| `tick()` | テスト開始後にワールドが進んだサーバーティック数を返す | `let t = world.tick();` |
| `log(message)` | テストランナーの標準エラー出力（テストログ）に1行出力する | `world.log("logged");` |
| `assert_entity_count(kind, position, radius, count)` | `position` から半径 `radius` 以内の `kind` の数が `count` と一致するか検証する | `world.assert_entity_count("minecraft:pig", ORIGIN, 4.0, 1)?;` |
| `assert_block(position, expected)` | 指定座標のブロックIDが `expected` と一致するか検証する | `world.assert_block(ORIGIN, "minecraft:stone")?;` |

### `TestDriver` トレイトと補助関数[^testing-rs]

`cargo test` でモックを作る際は `TestDriver` トレイトを実装します。最初の4つが必須メソッドで、残りの8つはデフォルト実装（`tick` は 0、`log` は何もしない、それ以外は `Err("unsupported: …")`）があるため必要なものだけ実装できます。なお、`engine::client_count` と `engine::run_with` は SDK の `test` フィーチャ（`yoake-wasm-sdk/test`）が有効なときだけ利用できます。

| 関数・メソッド名 | 動作 | 使用例 |
| --- | --- | --- |
| `TestDriver::place_block(position, block)` | 【必須】ブロック設置の処理を実装する | `driver.place_block(ORIGIN, "minecraft:stone")?;` |
| `TestDriver::block_at(position)` | 【必須】ブロックID取得の処理を実装する | `let id = driver.block_at(ORIGIN)?;` |
| `TestDriver::spawn_entity(kind, position)` | 【必須】エンティティ召喚の処理を実装する | `let id = driver.spawn_entity("minecraft:pig", ORIGIN)?;` |
| `TestDriver::run_ticks(ticks)` | 【必須】ティック進行の処理を実装する | `driver.run_ticks(2)?;` |
| `TestDriver::break_block(position)` | 【任意】ブロック破壊（既定: `Err("unsupported: break_block")`） | `driver.break_block(ORIGIN)?;` |
| `TestDriver::spawn_mannequin(name, position)` | 【任意】マネキン召喚（既定: `Err("unsupported: spawn_mannequin")`） | `let id = driver.spawn_mannequin("Tester", ORIGIN)?;` |
| `TestDriver::run_command(command)` | 【任意】コマンド実行（既定: `Err("unsupported: run_command")`） | `let ok = driver.run_command("say hi")?;` |
| `TestDriver::mod_call(target, method, payload)` | 【任意】サーバー MOD 呼び出し（既定: `Err("unsupported: mod_call")`） | `let res = driver.mod_call("demo", "echo", b"ping")?;` |
| `TestDriver::entities_near(position, radius)` | 【任意】周囲のエンティティ取得（既定: `Err("unsupported: entities_near")`） | `let list = driver.entities_near(ORIGIN, 4.0)?;` |
| `TestDriver::player_names()` | 【任意】プレイヤー名一覧取得（既定: `Err("unsupported: player_names")`） | `let names = driver.player_names()?;` |
| `TestDriver::tick()` | 【任意】経過ティック数の取得（既定: `0`） | `let t = driver.tick();` |
| `TestDriver::log(message)` | 【任意】ログ出力（既定: 何もしない） | `driver.log("logged");` |
| `TestCase::run(&self, driver)` | `TestDriver` を使って1件の `TestCase` を実行する | `case.run(&mut driver)?;` |
| `engine::client_count()` | 参加しているクライアント数を返す（1、または2クライアント時は2。SDKフィーチャ `test` が必要） | `let n = yoake_wasm_sdk::testing::engine::client_count();` |
| `engine::run_with(tests, name, driver)` | `tests` から `name` に一致するテストを探して `driver` で実行する（SDKフィーチャ `test` が必要） | `yoake_wasm_sdk::testing::engine::run_with(YOAKE_TESTS, "places_a_block", &mut driver)?;` |

---

## 6-3. 実行のしかた

### `yoake test` で実行する

プロジェクトフォルダで `yoake test` を実行すると、MOD 本体のビルドと `yoake check` に加えて、`--features tests` でテストモジュールをビルドし、ゲームを起動してテストを実行します。[^wasm-dev]

```sh
# PowerShell
$env:YOAKE_GAME='C:\path\to\horizon-client.exe'    # the game executable (Yoake build)
# macOS / Linux
export YOAKE_GAME=/path/to/horizon-client      # the game executable (Yoake build)

yoake test                                      # in the project folder: build, check, run the tests
yoake test --filter places --timeout-ms 60000 -- --resources /path/to/prepared/resources
```

`yoake test` の主なオプション:[^engine-test]
- `--game EXE`: ゲーム実行ファイルのパス（省略時は環境変数 `YOAKE_GAME` を使用）
- `--filter TEXT`: テスト名に `TEXT` を含むテストだけを実行
- `--timeout-ms N`: 1テストあたりの制限時間（ミリ秒。ゲームの `--yoake-test-timeout-ms` に渡され、省略時の既定値は `30000`）
- `--module WASM`: テストモジュールのパスを直接指定
- `--keep`: テスト用の一時フォルダを削除せずに残す
- `--clients N`: 参加するクライアント数（`N` は `1` か `2`。既定は `1`。`2` は [6-5](#6-5-2-クライアントのテスト) を見てください）
- `-- GAME-ARGS...`: `--` 以降の引数（`--resources <PATH>` や GPU不要の `--headless`、`--yoake-test-startup-seconds <N>` など）をそのままゲームに渡す

**出力内容**:
- 標準エラー出力（`stderr`）には、各テストの進行状況（`yoake test: ok   <name> (<ms> ms)` または `yoake test: FAIL <name> (<ms> ms)`）やエラー診断が出力されます。
- 標準出力（`stdout`）には、最後に**1行のJSON**が出力されます（全テスト成功時は終了コード `0`、それ以外は `1`）。ほかに `game` と `wasm_needs` も出ます。[^engine-test]

```json
{"ok": false, "built": true, "checked": true, "id": "my-mod", "version": "0.1.0",
 "tests": [{"name": "places_a_block", "ok": true, "ms": 412},
           {"name": "breaks_it", "ok": false, "ms": 380, "error": "block at BlockPos { .. }: expected ..."}],
 "errors": ["test breaks_it failed: block at BlockPos { .. }: expected ..."], "notes": []}
```

なお、`yoake dev --test` でも同じようにゲーム内でテストが実行され、結果の `tests` 配列が埋められます（`--test` なしの `yoake dev` はゲームを起動せず、`tests` は空のままです）。

### `horizon-client --yoake-test` を直接実行する

ゲーム実行ファイル側のインターフェースは `horizon-client --yoake-test <project folder | .yak> [options]` です（`--yoake-test` は**必ず最初の引数**として渡す必要があります）。

| オプション | 意味 |
| --- | --- |
| `--yoake-test-json FILE` | 標準出力に加えて、結果のJSONレポートを `FILE` にも書き出す |
| `--yoake-test-module FILE` | テストモジュールを指定する（プロジェクトフォルダの既定値は `bin/wasm/<id>-tests.wasm`。`.yak` パッケージにはテストが含まれないため必須） |
| `--yoake-test-with PATH` | テスト対象 MOD と一緒に読み込む追加パッケージ（フォルダまたは `.yak`）。複数回指定可能 |
| `--yoake-test-filter TEXT` | 名前が `TEXT` を含むテストのみを実行する |
| `--yoake-test-timeout-ms N` | 1テストあたりのタイムアウト（既定値 `30000` ミリ秒） |
| `--yoake-test-startup-seconds N` | ワーカーの起動からテスト開始までの猶予時間（既定値 `240` 秒） |
| `--yoake-test-keep` | 一時フォルダ（ゲームフォルダ・ワールド・ログ）を削除せず残し、JSONの `"kept"` にパスを記録する |
| `--yoake-test-clients=1\|2` | 参加するクライアント数（`2` で 2 クライアント。[6-5](#6-5-2-クライアントのテスト) を見てください） |

**出力内容**:
- 標準エラー出力（`stderr`）に各テストの `yoake test: ok  ` / `FAIL` 行を出し、標準出力（`stdout`）に `{"ok": ..., "id": ..., "version": ..., "clients": 1, "module": ..., "declared": ..., "tests": [...], "errors": [...]}` のJSONを1行出力します。
- 終了コードは `0`（全テスト成功）、`1`（テスト失敗）、`2`（コマンドライン不正やモジュール不在などでテスト未実行）です。

---

## 6-4. 平らなワールド（スーパーフラット）の前提と動作ルール

テストが走るワールドには以下の前提があります。[^world-facts]

1. **スーパーフラットの層と座標**:
   - シード値 `0`、構造物なし（structures off）のスーパーフラットワールドです。
   - 地面は、スーパーフラット標準の層構成（岩盤・土 2 層・草ブロック）で、`docs/yoake-wasm-dev.md` は正確な高さを固定していません（テストが高さに頼るときは `block_at` で読んでください）。このガイドでは、下から **`y = -64` が岩盤（bedrock）、`y = -63` と `y = -62` が土（dirt）、`y = -61` が草ブロック（grass）、`y = -60` 以上が空気（air）** です。サンプルが `y = -60` にブロックを置いているのは、草ブロックのすぐ上の空気層にあたるためです。
2. **初期状態とスポーン付近で作る**:
   - ワールドのオーナーはスポーン地点（`0, 0` 付近）にクリエイティブモード（`--creative`）で参加し、時刻は `6000`（`--time 6000`）、天候は晴れ（`--weather clear`）、描画距離は `4` チャンク（`--render-distance 4`）で始まります。テストで使うブロックやエンティティは `(0, 0)` 付近に作ってください。
   - オーナーは自動生成された名前（実測では **`PlayerNNNNN`** の形。要確認）で参加します。
3. **ワールドは凍結（freeze）されており、`run_ticks` でだけ進む**:
   - オーナーが参加したあと、チャンク読み込みのため実時間で2秒間だけワールドが動きます（ウォームアップ）。そのため、**テスト開始時点でワールドはすでにある程度のティック進んでいます**（実測では約 87〜103 ティックでしたが、このリポジトリのコードやドキュメントでは確認できていません（要確認）。`world.tick()` が `0` から始まるとは仮定しないでください）。
   - ウォームアップの後、**ワールドは凍結（freeze）**されます。凍結中は勝手にティックが進まず、`world.run_ticks(n)` を呼んだ回数だけ正確にサーバーティックが進みます（1 ティックごとの 50ms 待ちを飛ばして最速で実行されます）。
4. **テストごとに新しいワールド・新しいプロセスを起動するため時間がかかる**:
   - 前のテストの影響やハングアップを完全に切り離すため、**1テストごとに新しいワーカープロセスと一時ワールドが起動**します。
   - テスト本体の実行は数ミリ秒〜数百ミリ秒で終わりますが、ワーカーの起動とワールド準備には時間がかかります（Windowsデバッグビルド・`horizon-data.bin` キャッシュなしの実測で1テストあたり90〜100秒、リリースビルドやキャッシュありなら短縮）。
   - そのため、テストの数は絞り、1つのテストで関連する確認をまとめ、開発中は `yoake test --filter <名前>` で1つずつ実行するのがおすすめです。

---

## 6-5. 2 クライアントのテスト

`yoake test --clients 2` または `horizon-client --yoake-test <target> --yoake-test-clients=2`（`--yoake-test-clients 2`）を指定すると、1つのサーバーと2つのクライアントでマルチプレイテストを実行できます。クライアント `0` はワールドのオーナー（ワーカー自身）、クライアント `1` はループバック（`127.0.0.1`）経由で参加する2つ目のゲームプロセス（プレイヤー名 `TestB`）です。

```rust
fn a_change_reaches_the_second_client(world: &mut TestWorld<'_>) -> TestResult {
    let at = BlockPos { x: 3, y: -60, z: 3 };
    world.place_block(at, "minecraft:stone")?;
    world.run_ticks(2)?;                                       // the server sends the change
    world.client(1).wait_block(at, "minecraft:stone", 20_000)?; // real time, up to 20 s
    world.client(1).chat("hello")?;                             // client -> server -> everyone
    world.run_ticks(2)?;
    world.client(1).wait_chat("hello", 20_000)
}
```

### `world.client_count()` と `world.client(1)`（`ClientView`）の関数一覧[^testing-rs]

| 関数・シグネチャ | 動作 |
| --- | --- |
| `world.client_count(&mut self) -> u32` | 参加しているクライアント数を返す（通常は `1`、`--yoake-test-clients=2` 時は `2`） |
| `world.client(&mut self, index: u32) -> ClientView<'_, 'driver>` | クライアント `index`（`1` が2つ目のゲーム）の操作ハンドル `ClientView` を返す |
| `ClientView::call(&mut self, command: &str) -> Result<String, String>` | ゲームの制御プロトコル（`horizon-ctl` と同じ形式）の1行を送り、JSON応答を返す |
| `ClientView::block_at(&mut self, position: BlockPos) -> Result<String, String>` | そのクライアント自身が保持しているワールド上の `position` のブロックIDを返す |
| `ClientView::wait_block(&mut self, position: BlockPos, expected: &str, timeout_ms: u32) -> TestResult` | クライアント側の `position` のブロックが `expected` になるまで実時間で最大 `timeout_ms` ミリ秒待つ |
| `ClientView::chat(&mut self, text: &str) -> TestResult` | そのクライアントからチャットメッセージ `text`（先頭 `/` なしの平文）を送信する |
| `ClientView::wait_chat(&mut self, text: &str, timeout_ms: u32) -> TestResult` | そのクライアントの直近30件のチャット履歴に `text` が現れるまで実時間で最大 `timeout_ms` ミリ秒待つ |
| `ClientView::player_names(&mut self) -> Result<Vec<String>, String>` | クライアントが参加しているワールドのプレイヤー名一覧（オーナーと `TestB`）を返す |

（モック用の `TestDriver` トレイトにもデフォルト実装付きの `client_count`、`client_call`、`client_wait`、`client_player_names` が追加されています。）

- **待機関数（`wait_block` / `wait_chat`）と実時間**: サーバー側のワールドは `run_ticks` の間は凍結されていますが、2つ目のクライアントプロセスは凍結されていません。`wait_*` は**実時間（real time）**で50msごとに最大 `timeout_ms` ミリ秒まで確認を繰り返します。サーバーはティック進行時に変更を送信するため、待つ前に必ず `world.run_ticks(...)` を呼んでください。
- **サーバーからクライアントへのメッセージ**: サーバーコンソールの `say`・`me`、指名した相手への `msg` / `tell` / `w`、`tellraw @a "..."` は、いずれも 2 つ目のクライアントに届きます。[^wasm-dev]

---

## 6-6. よくある失敗と直し方

| よくあるエラーメッセージ・症状 | 原因と直し方 |
| --- | --- |
| `yoake test needs the game: pass --game <executable> or set YOAKE_GAME` | ゲーム実行ファイルの場所が指定されていません。環境変数 `YOAKE_GAME` に `horizon-client` のパスを設定するか、`yoake test --game <パス>` を渡してください。 |
| `the project has no tests: add a tests Cargo feature that calls export_tests!, or pass --module` | `Cargo.toml` に `[features] tests = ["yoake-wasm-sdk/test"]` がないか、テストモジュールが存在しません。フィーチャと `export_tests!(YOAKE_TESTS);` を追加してください。 |
| `the test module ... does not exist: run yoake build (or give --yoake-test-module)` / `the <side> module ... is missing: run yoake build first` | `horizon-client --yoake-test` を直接呼ぶ前にモジュールが `bin/wasm/` に配置されていません。先に `yoake build` を実行するか `--yoake-test-module` を指定してください。 |
| `a .yak carries no tests: pass the test module with --yoake-test-module` | `.yak` パッケージにはテストモジュールが含まれません。`.yak` を直接テストする場合は `--yoake-test-module bin/wasm/<id>-tests.wasm` を併せて指定してください。 |
| `... is a link or reparse point; move the project to a normal folder without links or reparse points` | シンボリックリンクやWindowsのジャンクション・同期フォルダ配下にプロジェクトがあります。通常のフォルダへ移動してください。 |
| `<option> cannot be used with --yoake-test: each test gets its own world and game folder` | `--world`、`--seed`、`--game-dir`、`--dedicated`、`--fullscreen` などはランナー自身が管理するため指定できません。バニラデータの指定には `-- --resources <PATH>` を使ってください。 |
| `--filter` の指定ミスで1件も一致しないのに終了コード `0`・`"ok": true` になる | 一致するテストが0件のときはエラーにならず、`"notes": ["no test matches the filter"]`（直接実行時は `"note": "no test matches the filter"`）で成功終了します。`tests` 配列が空になっていないか確認してください。[^coordinator] |
| `block at ...: expected "minecraft:oak_stairs[facing=north]", got "minecraft:oak_stairs"` | `block_at` / `assert_block` が返すのはプロパティを含まないブロックID（`minecraft:oak_stairs`）のみです。`assert_block` には `"minecraft:oak_stairs"` のようにIDだけを渡してください。 |
| `not a block state: ...` / `not a registry id (namespace:path): ...` / `not a player name (1 to 16 letters, digits, _): ...` | 引数の形式エラーです。エンティティ種別は `"minecraft:pig"` のように名前空間付き小文字、マネキン名は英数字と `_` の1〜16文字にしてください。 |
| `the test module trapped or ran out of time: ...` / `the worker did not finish within ... s and was killed` | テスト内で `panic!` や無限ループが起きたか、時間切れです。起動が遅い環境では `yoake test -- --yoake-test-startup-seconds 600`（`--` の後ろでゲームに渡す）、長いテストでは `--timeout-ms` を増やしてください。 |
| `entities_near` や `spawn_entity` を大量に呼ぶと途中で失敗する | ワールド凍結中は、`run_ticks` で進めない間は同じティックの予算（512〜1024 回）を使い回すため上限に達します。`world.run_ticks(1)?;` を呼んで次のティックへ進めてください。 |

---

## 6-7. 未実装の機能

以下の機能は、まだ実装されていないか、テストランナーの対象外です。[^limits]

- **クライアント専用フック（HUD 描画・キー入力など）の直接駆動と画面の検証（未実装）**:
  - テストはサーバー側で動くため、テストドライバからクライアント専用フック（HUD など）を直接駆動したり、スクリーンショット・描画フレームに対してアサーションを行ったりする機能はありません（未実装）。
  - 2 クライアントのテスト（[6-5](#6-5-2-クライアントのテスト)）では、`ClientView::call` で 2 つ目のクライアントへ `key`・`look`・`hotbar`・`mouse` などの制御プロトコルの 1 行を送れます。ただし、その結果として確かめられるのは、ドライバが読める範囲に限られます。
- **オーバーワールド以外のディメンション操作（未実装）**:
  - テストドライバが操作できるのはオーバーワールドのみです（未実装）。
- **ネイティブ（DLL）MOD パッケージのテスト実行**:
  - テストランナーの対象外です。起動するのはデータパッケージと Wasm パッケージのみで、ネイティブ（DLL）パッケージは実行されません（`native (DLL) packages are not run by the test runner`）。

---

## 次のステップ

テストの書き方が分かったら、次は [7. MOD を公開する（`07-publishing.md`）](./07-publishing.md) に進んで、バージョンの付け方・署名・更新の出し方を確認しましょう。

[^wasm-dev]: 確認元: `docs/yoake-wasm-dev.md`。
[^scaffold]: 確認元: `docs/yoake-wasm-dev.md` と `crates/yoake-cli/src/scaffold.rs`。
[^engine-test]: 確認元: `crates/yoake-cli/src/engine_test.rs`。
[^testing-rs]: 確認元: `crates/yoake-wasm-sdk/src/testing.rs`（`testing/engine.rs` を含む）。
[^world-facts]: 確認元: `docs/yoake-wasm-dev.md`（スーパーフラット・シード 0・構造物なし・クリエイティブ・時刻 6000・晴天・描画距離 4・2 秒のウォームアップ後に凍結）。層の高さ、ウォームアップ後のティック数、`PlayerNNNNN` の名前は `docs/release/1.0-test-plan.md` の記述によるもので、コードでは確認できていません（要確認）。
[^coordinator]: 確認元: `crates/yoake-test-runner/src/coordinator.rs`。
[^limits]: 確認元: `docs/yoake-wasm-dev.md` の「Limits and what is not covered」。
