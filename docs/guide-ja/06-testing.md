# 第6章 Modをゲームエンジンの中でテストする

前章までで、WebAssembly（Wasm）Modを作成してビルド・パッケージ化する流れを学びました。この章では、**ワールド操作のシナリオ（と、サーバー側 MOD への `mod_call`）を実際のゲームエンジン（`horizon-client`）の中で自動テストする方法**を解説します。なお、テストモジュールはサーバー側のワールドだけを操作する別コンポーネントとして動くため、**クライアント専用の MOD（HUD など）の動きは確かめられません**。

Yoakeのテスト機能（`yoake test`）を使うと、ウィンドウを表示しないオフスクリーン状態（`--offscreen`）でゲームが起動し、テストごとに毎回まっさらなスーパーフラットワールドを作ってテスト関数を実行してくれます。また、ゲームを起動せずに手元のモック（模擬ドライバ）だけでワールド操作のシナリオを素早く確かめることもできます。

---

## 1. テストを書く準備

### テストモジュールの仕組み

Yoakeのテストは、通常のMod本体（`client-mod` などのワールド）とは別のWebAssemblyコンポーネントである**テストモジュール（test module）**としてビルドされます（WITワールドは `crates/yoake-wasm/wit/yoake-test.wit` の `test-mod`）。

- 通常のビルドではMod本体（`bin/wasm/<id>.wasm`）が生成されます。
- Cargoフィーチャ `tests` を有効にしてビルドすると、同じクレートからテストモジュール（`bin/wasm/<id>-tests.wasm`）が生成されます。
- テストモジュールは `yak.toml` には記述せず、配布用の `.yak` パッケージにも含まれません。そのため、テスト用のコードが誤って配布用Modに混ざる心配がありません。

### `Cargo.toml` の設定（`tests` フィーチャ）

`yoake new` で作成したプロジェクトには最初から設定されていますが、手動で追加する場合は `Cargo.toml` に `tests = ["yoake-wasm-sdk/test"]` を定義します（出典: `docs/yoake-wasm-dev.md` / `crates/yoake-cli/src/scaffold.rs`）。

```toml
[features]
tests = ["yoake-wasm-sdk/test"]
```

これにより、テストビルド時のみSDKの `test` フィーチャ（`yoake_wasm_sdk::testing::engine` と `export_tests!` マクロ）が有効になります。

### `#[yoake_test]` と `export_tests!` でテストを登録する

テスト関数は `&mut TestWorld<'_>` を受け取り `TestResult`（`Result<(), String>`）を返す関数として書きます。`Ok(())` を返せば成功、`Err(メッセージ)` を返せば失敗です（`?` 演算子でドライバのエラーをそのまま返せます）。

以下は `docs/yoake-wasm-dev.md` の構成例です（`Starter` は自分の MOD の型名に置き換えます）。

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

`yoake new` のテンプレート（`crates/yoake-cli/src/scaffold.rs`）のように、テスト関数には `#[yoake_wasm_sdk::yoake_test]`（または `use yoake_wasm_sdk::yoake_test;` して `#[yoake_test]`）属性を付けることができます。

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

※ `yoake new` のひな形では `y: 64`（空中の座標）が使われていますが、地面のすぐ上に置く場合は上の例のように `y: -60` 付近を使います。

- **`#[yoake_test]`**: テスト関数であることを示すマーカー属性です（引数は取らず、引数を渡すと `"yoake_test does not accept arguments"` でコンパイルエラーになります）。この属性単体では自動登録されないため、必ず `YOAKE_TESTS` スライス（`&[TestCase]`）にも登録してください。
- **`export_tests!(YOAKE_TESTS)`**: `feature = "tests"` のときに `YOAKE_TESTS` をテストモジュールの `tests` / `run-test` エクスポートとして公開します。
- **テスト名のルール**: 1〜128文字、モジュール内で重複なし、最大1024個までです。またコマンドライン引数として渡されるため、先頭を `-` にすることはできません。

---

## 2. `TestWorld` と `TestDriver` の全関数リファレンス

テスト関数が受け取る `TestWorld` は、裏側の `TestDriver` トレイト（実ゲームでは `engine::HostDriver`、ローカルの `cargo test` では自作モック）へ処理を渡します。すべての操作は統合サーバー（Integrated Server）スレッドのティック間で実行されます。

### `TestWorld` の公開メソッド一覧（`crates/yoake-wasm-sdk/src/testing.rs`）

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
| `mod_call(target, method, payload)` | 読み込み済みのサーバーモジュール（テスト対象Mod自身、または `--yoake-test-with` で追加したMod）の `handle-call` を呼び出す | `let echoed = world.mod_call("demo", "echo", b"ping")?;` |
| `entities_near(position, radius)` | `position` から半径 `radius` ブロック以内のエンティティ種別一覧（最大512件）を返す | `let kinds = world.entities_near(ORIGIN, 4.0)?;` |
| `player_names()` | サーバー上のプレイヤー名一覧（ワールドのオーナーが先頭）を返す | `let names = world.player_names()?;` |
| `tick()` | テスト開始後にワールドが進んだサーバーティック数を返す | `let t = world.tick();` |
| `log(message)` | テストランナーの標準エラー出力（テストログ）に1行出力する | `world.log("logged");` |
| `assert_entity_count(kind, position, radius, count)` | `position` から半径 `radius` 以内の `kind` の数が `count` と一致するか検証する | `world.assert_entity_count("minecraft:pig", ORIGIN, 4.0, 1)?;` |
| `assert_block(position, expected)` | 指定座標のブロックIDが `expected` と一致するか検証する | `world.assert_block(ORIGIN, "minecraft:stone")?;` |

### `TestDriver` トレイトと補助関数（`testing.rs` / `testing/engine.rs`）

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
| `TestDriver::mod_call(target, method, payload)` | 【任意】サーバーMod呼び出し（既定: `Err("unsupported: mod_call")`） | `let res = driver.mod_call("demo", "echo", b"ping")?;` |
| `TestDriver::entities_near(position, radius)` | 【任意】周囲のエンティティ取得（既定: `Err("unsupported: entities_near")`） | `let list = driver.entities_near(ORIGIN, 4.0)?;` |
| `TestDriver::player_names()` | 【任意】プレイヤー名一覧取得（既定: `Err("unsupported: player_names")`） | `let names = driver.player_names()?;` |
| `TestDriver::tick()` | 【任意】経過ティック数の取得（既定: `0`） | `let t = driver.tick();` |
| `TestDriver::log(message)` | 【任意】ログ出力（既定: 何もしない） | `driver.log("logged");` |
| `TestCase::run(&self, driver)` | `TestDriver` を使って1件の `TestCase` を実行する | `case.run(&mut driver)?;` |
| `engine::client_count()` | 参加しているクライアント数を返す（現在は `1`。SDKフィーチャ `test` が必要） | `let n = yoake_wasm_sdk::testing::engine::client_count();` |
| `engine::run_with(tests, name, driver)` | `tests` から `name` に一致するテストを探して `driver` で実行する（SDKフィーチャ `test` が必要） | `yoake_wasm_sdk::testing::engine::run_with(YOAKE_TESTS, "places_a_block", &mut driver)?;` |

---

## 3. 実行のしかた

### `yoake test` で実行する

プロジェクトフォルダで `yoake test` を実行すると、Mod本体のビルドと `yoake check` に加えて、`--features tests` でテストモジュールをビルドし、ゲームを起動してテストを実行します（出典: `docs/yoake-wasm-dev.md`）。

```sh
export YOAKE_GAME=/path/to/horizon-client      # the game executable (Yoake build)
yoake test                                      # in the project folder: build, check, run the tests
yoake test --filter places --timeout-ms 60000 -- --resources /path/to/prepared/resources
```

`yoake test` の主なオプション（`crates/yoake-cli/src/engine_test.rs`）:
- `--game EXE`: ゲーム実行ファイルのパス（省略時は環境変数 `YOAKE_GAME` を使用）
- `--filter TEXT`: テスト名に `TEXT` を含むテストだけを実行
- `--timeout-ms N`: 1テストあたりの制限時間（ミリ秒。ゲームの `--yoake-test-timeout-ms` に渡され、省略時の既定値は `30000`）
- `--module WASM`: テストモジュールのパスを直接指定
- `--keep`: テスト用の一時フォルダを削除せずに残す
- `-- GAME-ARGS...`: `--` 以降の引数（`--resources <PATH>` や GPU不要の `--headless`、`--yoake-test-startup-seconds <N>` など）をそのままゲームに渡す

**出力内容**:
- 標準エラー出力（`stderr`）には、各テストの進行状況（`yoake test: ok   <name> (<ms> ms)` または `yoake test: FAIL <name> (<ms> ms)`）やエラー診断が出力されます。
- 標準出力（`stdout`）には、最後に**1行のJSON**が出力されます（全テスト成功時は終了コード `0`、それ以外は `1`）。ほかに `game` と `wasm_needs` も出ます（`crates/yoake-cli/src/engine_test.rs`）。

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
| `--yoake-test-module FILE` | テストモジュールを指定する（プロジェクトフォルダの既定値は `bin/wasm/<id>-tests.wasm`。`.yak` にはテストが含まれないため必須） |
| `--yoake-test-with PATH` | テスト対象Modと一緒に読み込む追加パッケージ（フォルダまたは `.yak`）。複数回指定可能 |
| `--yoake-test-filter TEXT` | 名前が `TEXT` を含むテストのみを実行する |
| `--yoake-test-timeout-ms N` | 1テストあたりのタイムアウト（既定値 `30000` ミリ秒） |
| `--yoake-test-startup-seconds N` | ワーカーの起動からテスト開始までの猶予時間（既定値 `240` 秒） |
| `--yoake-test-keep` | 一時フォルダ（ゲームフォルダ・ワールド・ログ）を削除せず残し、JSONの `"kept"` にパスを記録する |
| `--yoake-test-clients=1\|2` | 参加するクライアント数（`2` は未実装） |

**出力内容**:
- 標準エラー出力（`stderr`）に各テストの `yoake test: ok  ` / `FAIL` 行を出し、標準出力（`stdout`）に `{"ok": ..., "id": ..., "version": ..., "clients": 1, "module": ..., "declared": ..., "tests": [...], "errors": [...]}` のJSONを1行出力します。
- 終了コードは `0`（全テスト成功）、`1`（テスト失敗）、`2`（コマンドライン不正やモジュール不在などでテスト未実行）です。

---

## 4. 平らなワールド（スーパーフラット）の前提と動作ルール

テストが走るワールドには以下の前提があります（出典: `docs/yoake-wasm-dev.md`「Declaring tests: a test module」および「How a run works」）。

1. **スーパーフラットの層と座標**:
   - シード値 `0`、構造物なし（structures off）のスーパーフラットワールドです。
   - 地面は下から **岩盤 1・土 2・草 1** です。ドキュメントで高さは固定されていないので `block_at` で確かめます（`the exact height is not pinned here; read it with block_at if a test depends on it`）。サンプルは `y = -60` に置いています（おそらく岩盤 `-64`・土 `-63`/`-62`・草 `-61`、未確定）。
2. **初期状態とスポーン付近で作る**:
   - ワールドのオーナーはスポーン地点（`0, 0` 付近）にクリエイティブモード（`--creative`）で参加し、時刻は `6000`（`--time 6000`）、天候は晴れ（`--weather clear`）、描画距離は `4` チャンク（`--render-distance 4`）で始まります。テストで使うブロックやエンティティは `(0, 0)` 付近に作ってください。
3. **ワールドは凍結（freeze）されており、`run_ticks` でだけ進む**:
   - オーナーが参加したあと、チャンク読み込みのため実時間で2秒間だけワールドが動きます（ウォームアップ）。
   - その後、**ワールドは凍結（freeze）**されます。凍結中は勝手にティックが進まず、`world.run_ticks(n)` を呼んだ回数だけ正確にサーバーティックが進みます（1 ティックごとの 50ms 待ちを飛ばして最速で実行されます）。
4. **テストごとに新しいワールド・新しいプロセスを起動するため時間がかかる**:
   - 前のテストの影響やハングアップを完全に切り離すため、**1テストごとに新しいワーカープロセスと一時ワールドが起動**します。
   - テスト本体の実行は数ミリ秒〜数百ミリ秒で終わりますが、ワーカーの起動とワールド準備には時間がかかります（Windowsデバッグビルド・`horizon-data.bin` キャッシュなしの実測で1テストあたり90〜100秒、リリースビルドやキャッシュありなら短縮）。
   - そのため、テストの数は絞り、1つのテストで関連する確認をまとめ、開発中は `yoake test --filter <名前>` で1つずつ実行するのがおすすめです。

---

## 5. よくある失敗と直し方

| よくあるエラーメッセージ・症状 | 原因と直し方 |
| --- | --- |
| `yoake test needs the game: pass --game <executable> or set YOAKE_GAME` | ゲーム実行ファイルの場所が指定されていません。環境変数 `YOAKE_GAME` に `horizon-client` のパスを設定するか、`yoake test --game <パス>` を渡してください。 |
| `the project has no tests: add a tests Cargo feature that calls export_tests!, or pass --module` | `Cargo.toml` に `[features] tests = ["yoake-wasm-sdk/test"]` がないか、テストモジュールが存在しません。フィーチャと `export_tests!(YOAKE_TESTS);` を追加してください。 |
| `the test module ... does not exist: run yoake build (or give --yoake-test-module)` / `the <side> module ... is missing: run yoake build first` | `horizon-client --yoake-test` を直接呼ぶ前にモジュールが `bin/wasm/` に配置されていません。先に `yoake build` を実行するか `--yoake-test-module` を指定してください。 |
| `a .yak carries no tests: pass the test module with --yoake-test-module` | `.yak` ファイルにはテストモジュールが同梱されません。`.yak` を直接テストする場合は `--yoake-test-module bin/wasm/<id>-tests.wasm` を併せて指定してください。 |
| `... is a link or reparse point; move the project to a normal folder without links or reparse points` | シンボリックリンクやWindowsのジャンクション・同期フォルダ配下にプロジェクトがあります。通常のフォルダへ移動してください。 |
| `<option> cannot be used with --yoake-test: each test gets its own world and game folder` | `--world`、`--seed`、`--game-dir`、`--dedicated`、`--fullscreen` などはランナー自身が管理するため指定できません。バニラデータの指定には `-- --resources <PATH>` を使ってください。 |
| `--filter` の指定ミスで1件も一致しないのに終了コード `0`・`"ok": true` になる | 一致するテストが0件のときはエラーにならず、`"notes": ["no test matches the filter"]`（直接実行時は `"note": "no test matches the filter"`）で成功終了します（`coordinator.rs`）。`tests` 配列が空になっていないか確認してください。 |
| `block at ...: expected "minecraft:oak_stairs[facing=north]", got "minecraft:oak_stairs"` | `block_at` / `assert_block` が返すのはプロパティを含まないブロックID（`minecraft:oak_stairs`）のみです。`assert_block` には `"minecraft:oak_stairs"` のようにIDだけを渡してください。 |
| `not a block state: ...` / `not a registry id (namespace:path): ...` / `not a player name (1 to 16 letters, digits, _): ...` | 引数の形式エラーです。エンティティ種別は `"minecraft:pig"` のように名前空間付き小文字、マネキン名は英数字と `_` の1〜16文字にしてください。 |
| `the test module trapped or ran out of time: ...` / `the worker did not finish within ... s and was killed` | テスト内で `panic!` や無限ループが起きたか、時間切れです。起動が遅い環境では `yoake test -- --yoake-test-startup-seconds 600`（`--` の後ろでゲームに渡す）、長いテストでは `--timeout-ms` を増やしてください。 |
| `entities_near` や `spawn_entity` を大量に呼ぶと途中で失敗する | ワールド凍結中は、`run_ticks` で進めない間は同じティックの予算（512〜1024 回）を使い回すため上限に達します。`world.run_ticks(1)?;` を呼んで次のティックへ進めてください。 |

---

## 6. 未実装の機能（未実装）

以下の機能は現在のブランチではまだ実装されていません（出典: `docs/yoake-wasm-dev.md`「Two clients (`--yoake-test-clients=2`): design, not implemented」および「Limits and what is not covered」）。

- **2クライアントでのマルチプレイテスト（`--yoake-test-clients=2`）（未実装）**:
  - コマンドラインオプション `--yoake-test-clients=2` のパースと `yoake_wasm_sdk::testing::engine::client_count()` 関数自体は存在しますが、`2` を指定して実行すると終了コード `2` とエラー `"--yoake-test-clients=2 is not implemented yet: the two-client mode is designed in docs/yoake-wasm-dev.md but only one client runs"` で終了します。
  - 2つ目のクライアントを操作する `world.client(1)` ハンドルや WIT の `client-call(index, command)` も設計段階であり（未実装）です。
- **クライアント専用フック（HUD描画・キー入力など）の操作や画面の検証（未実装）**:
  - テストはシングルプレイワールドのサーバー側で動くため、キー入力の送信やスクリーンショット・描画フレームに対するアサーション機能はありません（未実装）。
- **オーバーワールド以外のディメンション操作（未実装）**:
  - テストドライバが操作できるのはオーバーワールドのみです（未実装）。
- **ネイティブ（DLL）Modパッケージのテスト実行（未実装）**:
  - テストランナーが起動するのはデータパッケージとWasmパッケージのみで、ネイティブ（DLL）パッケージは実行されません（`native (DLL) packages are not run by the test runner`）。
