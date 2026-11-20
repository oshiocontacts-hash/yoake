# 4. サンプル MOD 集を読む

`crates/yoake-wasm-sdk/examples/` には、目的別の 19 個のサンプル MOD が入っています（`hello` と `hello-std` は 4-1 の表で 1 行にまとめています）。
自分が作りたい機能に近いサンプルを開いて、`yak.toml` と `src/lib.rs` を真似するのが一番の近道です。

---

## 4-1. サンプル一覧（何を見せているか・使うフックとホストサービス）

| サンプル名 | WIT ワールド（MOD の種類） | 何を見せているか | 主なフック（`hook::*`） | 主なホストサービス |
|---|---|---|---|---|
| `hello`（および `hello-std`） | `client-mod` | 最小のクライアント MOD。ティック数を数えて HUD に背景矩形と文字を描画し、設定ボタン名を返す（`hello` は `#![no_std]`、`hello-std` は `std` 使用） | `TICK`, `HUD`, `SETTINGS`（`hello-std` は `TICK`, `KEY`, `HUD`） | `yoake_wasm_sdk::log`, `hud::rect`, `hud::text` |
| `hud-item` | `client-full` | 毎フレーム HUD に半透明パネル・`minecraft:diamond` のアイテムアイコン・ラベル文字を描画する | `HUD` | なし（`et::HudOutcome` に `Rect`, `Item`, `Text` を詰めて返す） |
| `server-storage` | `server-full` | `/visits` コマンドを追加し、プレイヤー UUID ごとの訪問回数を `World` スコープのストレージに保存して返答する | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::storage_get`, `services::storage_put`, `services::tell` |
| `config-demo` | `client-full` | `yak.toml` の `[config]` で定義した文字と色を読み取り、設定変更時に `config_changed` で HUD 表示を更新する | `HUD`, `CONFIG_CHANGED`（＋ `init`） | `services::config_get` |
| `permission-demo` | `server-full` | `/secret` コマンドを追加し、`has_permission` で `"secret.access"` 権限ノードの有無を判定して返答を変える | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::has_permission`, `services::tell` |
| `full-demo` | `client-full` / `server-full` | クライアント（HUD・ホットバー非表示・ツールチップ追加・FOV 変更）、サーバー（`/wasmping`・`/wasmsvc` で各種サービス巡回）、別 MOD 連携（`peer`）の総合デモ | client: `HUD`, `GUI_LAYER_PRE`, `TOOLTIP`, `COMPUTE_FOV` / server: `REGISTER_COMMANDS`, `PERFORM_COMMAND`, `SERVER_TICK` / peer: `handle_call` | `tell`, `give_items`, `break_block_as`, `edit_blocks`, `run_command`, `mod_call` ほか多数 |
| `full-services` | `client-full` / `server-full` | 設定・ストレージ（通常および分割 blob）・権限ノードの確認の一連の操作と、サービス権限が未宣言のときに `"permission denied: ..."` で拒否される動作の検証 | `CONFIG_CHANGED`（＋ client `init` / server `handle_call`） | `config_get`, `config_set`, `storage_get`, `storage_put`, `storage_delete`, `storage_blob_begin/write/commit/abort/read`, `has_permission` |
| `mod-call-pair` | `server-full` | `caller` の `/ping <text>` から `responder` の `handle_call` を呼び、2 つの MOD で UTF-8 テキストをやり取りする | `REGISTER_COMMANDS`, `PERFORM_COMMAND` / `handle_call` | `services::mod_call`, `services::tell` |
| `chest-count` | `server-full` | `/chestcount <x> <y> <z>` で指定した座標のコンテナを読み、アイテム ID ごとの合計を返答する | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::container_slots`, `services::tell` |
| `item-trade` | `server-full` | `/trade` で丸石をちょうど 16 個、全部取れるときだけ取り出し（足りなければ何も取らない）、ダイヤモンド 1 個を与える | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::inventory_apply`（`Give`）, `ext::take`, `ext::take_exact`, `services::tell` |
| `player-vitals` | `server-full` | `/vitals` でプレイヤーの体力と `minecraft:story/root` 実績の `known` / `done` を返答する | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::advancement_progress`, `services::attribute_get`, `services::tell` |
| `tested-counter` | `client-mod` | ティック数を数えて HUD に表示する小さなクライアント MOD に、`#[yoake_test]` によるゲームエンジン内のテストを組み合わせた入門例 | `TICK`, `HUD` | `hud::text` |
| `content-demo` | `server-full` | データだけで定義したブロック・アイテム・エフェクト・レシピ・ルートテーブル・モブ・サウンド・クリエイティブタブに、サーバーモジュールが反応する（`content_demo:gem_cow` を右クリックで鳴き声、倒した数をワールドごとに数える） | `INTERACT_ENTITY`, `MOB_KILLED`, `ENTITY_TICK_POST` | `services::acting_player`, `services::play_sound_at`, `services::tell`, `ext::attachment_get`, `ext::attachment_set` |
| `mixin-demo` | `inject-mod` | 落下ダメージの半減、骨粉の成長量の 2 倍化、チャットの記録（[8 章](./08-mixin.md)） | なし（フックではなく Mixin の HEAD / RETURN で割り込む） | なし（`println!` のログのみ） |
| `test-demo` | `test-mod`（テストモジュール） | テストランナー自体を試すための 6 つのテスト（成功・失敗・トラップ・時間切れ・ドライバの全呼び出し・2 クライアント）（[6 章](./06-testing.md)） | なし | `TestWorld` の各呼び出し |
| `client-services-demo` | `client-full` | クライアントのサービス（キーバインドの登録と状態の取得、ワールド内の線の描画、ポストシェーダーの登録、書き込み可能な GUI テクスチャーの作成と範囲書き込み、IME の有効化と候補位置）を `hud` フックの中で呼び、正しい引数では成功し、不正な入力（無効なシェーダー、範囲外の書き込み）ではエラーが返ることを確かめる。ランタイムの統合テストで使うゲスト | `HUD` | `services::register_key_binding`, `services::key_binding_state`, `services::draw_world_line`, `services::register_post_shader`, `hud::GuiTexture::create`, `services::gui_texture_write`, `services::ime_set_allowed`, `services::ime_set_cursor_area` |
| `gpu-demo` | `client-full` | GPU の口（`gpu` モジュール）の例。テクスチャー付きメッシュ、コンピュートで更新するパーティクル、深度を使ったワールド描画後の色付けを描く。`--features verify` / `reload` / `trap` で検証用のビルドもできる（Cargo の feature） | `HUD` | `gpu::{Shader, RenderPipeline, ComputePipeline, Buffer, BindGroup}` ほか |
| `ime` | `client-full` | MOD 画面の IME を有効化し、変換途中の文字列とカーソル範囲、確定文字列を受け取り、候補ウィンドウの位置を GUI 座標で設定する（[9 章](./10-ime.md)） | `SCREEN_INIT_POST`, `FRAME`, `IME_PREEDIT`, `IME_COMMIT` | `services::ime_set_allowed`, `services::ime_set_cursor_area` |

---

## 4-2. 各サンプルのポイントとコード抜粋

### ① `hello` / `hud-item`（HUD に文字やアイテムを描く）

- 19 個の基本フックだけで足りる場合は `hello` のように `yoake_wasm_sdk::Mod` と `export_mod!` を使います。
- アイテムアイコン（`HudCommand::Item`）やツールチップ変更などフルカタログ（client-full / server-full）の機能を使う場合は、`hud-item` のように `yak.toml` に `world = "client-full"` を書き、`ClientMod` と `export_client_full!` を使います。

`hud-item` の実装より抜粋:[^hud-item]

```rust
        // 2. A 16x16 item icon for "minecraft:diamond".
        //    `components` holds optional binary item component data; an empty
        //    Vec means a default stack with no extra components.
        let icon = et::HudCommand::Item(et::HudItem {
            stack: et::ItemStack {
                id: "minecraft:diamond".to_owned(),
                count: 1,
                components: Vec::new(),
            },
            x: panel_x + 4.0,
            y: panel_y + 4.0,
            decorations: true,
        });
```

### ② `server-storage`（コマンド追加とデータ保存）

サーバー側で動く MOD は `yak.toml` の `[wasm]` に `server = "bin/wasm/..."` を指定します（サーバーモジュールの WIT ワールドは `server-full` です。Mixin の `inject-mod` を除きます。[8 章](./08-mixin.md) を参照）。
`storage_get` や `storage_put` を使うには、`yak.toml` の `[wasm]` に `permissions = ["service.storage"]` が必要です。

`server-storage` の実装より抜粋:[^server-storage]

```rust
    fn register_commands(&mut self, mut commands: Vec<et::NativeCommand>) -> Vec<et::NativeCommand> {
        commands.push(et::NativeCommand {
            usage: "visits".to_owned(),
            level: 0,
        });
        commands
    }
```

```rust
        let current = match services::storage_get(&key, types::StorageScope::World) {
            Ok(Some(bytes)) => decode_u64(&bytes),
            Ok(None) => 0,
            Err(err) => {
                let _ = services::tell(&format!("Failed to read visits: {err}"), &uuid);
                return true;
            }
        };

        let next = current.saturating_add(1);
        if let Err(err) =
            services::storage_put(&key, types::StorageScope::World, &next.to_le_bytes())
        {
            let _ = services::tell(&format!("Failed to save visits: {err}"), &uuid);
            return true;
        }
```

- ストレージスコープ（`StorageScope`）は、サーバー側ではセーブデータのワールドごとの `World`（`<world>/yoake-data/<id>/`）と全ワールド共通の `Global`（`<game>/yoake-data/<id>/global/`）、クライアント側では `Client`（`<game>/yoake-data/<id>/client/`）を選べます。`World` と `Global` はサーバー専用、`Client` はクライアント専用で、各 MOD は自分のパッケージのストアだけにアクセスできます。`World` のストレージが使えるのは `world_lifecycle(WorldOpenReady)` からで、`server_about_to_start` と最初のレベル／チャンク読み込みフックでは使えません（そこでは `Global` を使います）。[^storage]

各ストアフォルダには、ホストが生成したキーごとの `entry-*.bin` と `store.lock` があり、全キーを 1 つの `store.bin` にまとめる方式ではありません。ロックはストアを開いている間保持され、別プロセスから同じストアを開くとエラーになります。1 キーの変更ではその値とメタデータだけを書き込み、一時ファイルへ書いて `fsync` してから rename で置き換えます。`put`、`delete`、blob の commit が成功を返すまでに永続化が完了します。開いたときに残っている古い一時ファイルは削除され、Unix では rename 時に親ディレクトリも同期します。

各ストアの容量上限は、シリアル化されたメタデータと予約済みの未完了 blob アップロードを含めて 256 MiB、キー数は最大 65,536 です。メモリに常駐するのはキーと長さだけで、値と blob のスライスは必要に応じて読み込みます。書き込み量は MOD ごと・全スコープ共通のトークンバケットで制限され、設定の書き込みも含めて継続 8 MiB/秒、バースト 32 MiB です。さらにストレージ操作は 1 回あたり最低 4 KiB 分を消費し、50 ms のフックウィンドウあたり最大 64 操作までです。どちらかを超えるとエラーを返し、超過が続くと MOD が無効化されます。

通常の読み書きと blob のチャンクは、それぞれ最大 1 MiB です。`storage_blob_begin` は容量を予約してパッケージ所有のアップロード ID を返し、未完了のアップロードは各ストアで最大 64 件、MOD ごとに全スコープ合計 32 MiB までです。バッファは begin 時ではなく write 時に必要に応じて確保されます。チャンクは順不同で書き込めますが、commit には対象全体が欠けなくそろっている必要があります。commit に失敗してもアップロードは残るため、続きを書いて再試行するか abort できます。`storage_blob_read` は commit 済みデータの範囲を上限付きで読みます。アップロードは永続化されず、ワールドのアップロードはワールド変更時やモジュールインスタンスの作成・削除・無効化・トラップ時に破棄されます。

ストアを最初に開くとき、旧形式のデータは容量上限内で自動的にインポートされます。[^storage-legacy] 破損した `entry-*.bin` は `*.bin.corrupt` に名前を変えて隔離され（1 ストアにつき最大 16 個。超えた分は削除）、正常なエントリは引き続き使えます。旧形式の取り込み中に壊れた、または容量を超えたレコードは警告を出して飛ばします。再試行が必要な I/O 失敗があった場合は移行マーカーを書かず、ストアを開き直したときに再試行します。

> [!NOTE]
> ディスク上の形式は内部仕様であり、今後のバージョンで変わる可能性があります。ファイルを直接編集したり、
> 形式に依存した道具を作ったりせず、必ず `storage_get` / `storage_put` と blob 用 API（`storage_blob_*`）を使ってください（[7. MOD を公開する](./07-publishing.md) の「やってはいけないこと」も参照）。

### ③ `mod-call-pair`（別の MOD を呼び出す）

`caller` と `responder` は別々の `.yak` パッケージなので、両方をインストールします。`caller` は `/ping <text>` の文字列をバイト列にして `services::mod_call("echo", ..., "mod-call-responder")` で送り、`responder` は `handle_call` で `pong` の返事を返します。呼び出し先には `yak.toml` のパッケージ ID を使います。`yak.toml` に呼び出し先を書く欄はなく、追加のサービス権限もありません。`responder` をインストールして有効にしていない場合、プレイヤーには `no package ... with a server module` というエラーが表示されます。どちら側も、受け取ったバイト列を UTF-8 へ変換できなければ、panic せずにエラーとして扱います。

### ④ `chest-count`（コンテナのアイテムを数える）

`/chestcount <x> <y> <z>` は、コマンドを実行したプレイヤーと同じディメンションの指定座標を `services::container_slots` で読みます。各スロットの `Option<types::ItemStack>` から中身だけを取り出し、アイテム ID ごとに `count` を合計します。ラージチェストは両半分のスロットが返ります。対象はコマンドを実行したプレイヤーが今いるディメンションのコンテナだけです。チェスト以外のコンテナにも使えますが、コンテナが無い座標や読み込まれていないチャンクではエラーになり、コンポスターもコンテナとしては扱われません。

### ⑤ `item-trade`（アイテムを交換する）

`/trade` は、先に `services::inventory_apply` の `Give` で `minecraft:diamond` を 1 個与えます（メインスロットだけが対象で、全量入るか何も入らないかのどちらかです）。入らなければ「インベントリがいっぱい」と知らせて終了し、何も変わりません。入ったら `ext::take` で `minecraft:cobblestone` を 16 個ちょうど取り出します。これも全量または無しの操作で、オフハンドも対象です。取り出しに失敗した（丸石が足りない）ときは、いま与えたダイヤモンドを `ext::take_exact` で取り戻して取引を取り消します。順序を「与えてから取る」にしているので、入力を返す処理は要りません。`ext::take` は名前やタグなどのコンポーネントが付いた丸石も取りますが、`ext::take_exact` はアイテム・個数・コンポーネントが完全に一致するスタックだけを取ります。

### ⑥ `player-vitals`（体力と実績の進捗を読む）

`/vitals` はコマンドフックの `player.health` から現在の体力を、`services::attribute_get("minecraft:max_health", ...)` から最大体力を読み、`services::advancement_progress` で `minecraft:story/root` の状態を読みます。返ってくる `types::AdvancementState` の `known` は実績を認識しているか、`done` は完了しているかを表します。実績や最大体力の読み取りに失敗した場合も、取得できている体力は返答します。

### ⑦ `content-demo`（データ定義のコンテンツにサーバーモジュールが反応する）

ブロック・アイテム・エフェクト・レシピ・ルートテーブル・モブ・サウンド・クリエイティブタブは `content/` の中のデータだけで定義し、サーバーモジュールはそれに反応するだけです。`content_demo:gem_cow` を右クリックすると `interact_entity` で独自のサウンドを鳴らしてプレイヤーに知らせ、倒すと `mob_killed` でワールドごとの撃破数をデータ添付（`ext::attachment_get` / `ext::attachment_set`）に保存します。`yak.toml` の `id` は `content_demo` で、`bin/wasm/content-demo-server.wasm` をサーバーモジュールとして宣言します。

### ⑧ `mixin-demo`（エンジン関数に割り込む）

`inject-mod` の WIT ワールドで、落下ダメージの半減、骨粉の成長量の 2 倍化、チャットの記録を行います。通常のフックではなく Mixin の割り込みなので、書き方は [8. Yoake Mixin 入門（`08-mixin.md`）](./08-mixin.md) で説明します。

### ⑨ `tested-counter` と `test-demo`（ゲームエンジン内のテスト）

`tested-counter` は、ティック数を HUD に出すクライアント MOD と `#[yoake_test]` のテストを同じクレートに入れた入門例です。`test-demo` はテストランナー自体を試すためのテストモジュールで、成功・失敗・トラップ・時間切れ・ドライバの全呼び出し・2 クライアントのテストが入っています。どちらも [6. MOD をゲームエンジンの中でテストする（`06-testing.md`）](./06-testing.md) と合わせて読んでください。

### ⑩ `client-services-demo` と `gpu-demo`（クライアント側の新しいサービスと GPU）

どちらも `client-full` ワールドのクライアント MOD で、`export_client_full!` を使います。サンプルのフォルダに `yak.toml` はなく、ランタイムのテストがビルドした `.wasm` を直接読み込みます。実際に配る MOD に使うときは、`yak.toml` の `[wasm] world` に `"client-full"` を書いてください。

- `client-services-demo`: `hud` フックの中でキーバインド、ワールド内の線、ポストシェーダー、GUI テクスチャー、IME の各サービスを呼びます。成功する呼び出しと、エラーが返るべき呼び出し（無効なシェーダー、テクスチャーの範囲外への書き込み）を両方試すので、サービスの使い方とエラーの扱いを確かめる例になります。例は `[wasm] permissions` を宣言しません。
- `gpu-demo`: `gpu` モジュールで WGSL シェーダーからレンダー／コンピュートのパイプラインを作る例です。`gpu` の呼び出しには `[wasm] permissions` の `"service.gpu"` が必要です（4-3 参照）。使い方の詳細は `docs/yoake-wasm-dev.md` の GPU の節を参照してください。

### ⑪ `ime`（MOD 画面の日本語入力）

`ime` は自分の MOD 画面で IME を有効にし、preedit・カーソル範囲・確定文字列を受け取る `client-full` の例です。候補ウィンドウの矩形を GUI スケーリング済み座標で設定します。使い方と既存の文字フックとの二重入力を防ぐ注意は [9. MOD 画面で IME（日本語入力）を使う](./10-ime.md) を参照してください。

---

## 4-3. `yak.toml` での `[wasm] permissions`・`[config]`・`[permissions."<node>"]` の書き方

MOD が特別な機能（設定の読み書き、ストレージ保存、権限ノードのチェック、サーバーコマンド実行など）を使うときは、`yak.toml` に宣言を書きます。

### `[wasm] permissions` の種類

指定可能なサービス権限（capability、`[wasm] permissions`）は次の 7 つです。[^manifest]

- `"service.chat"`: クライアントモジュールが Mixin でチャット／コマンドの文字を書き換えるサービス権限（`wasm.client` が必須で、`service.commands` と合わせて使います。[8 章](./08-mixin.md)）
- `"service.commands"`: サーバーモジュールからの `run_command` の呼び出し（`wasm.server` が必須。`inject-mod` はサーバーモジュールが無くてもよい）。Mixin でコマンド・チャットのサイトに割り込むときにも必要です（[8 章](./08-mixin.md)）
- `"service.config"`: `config_get` / `config_set` / `config_dir` の呼び出しと、同期設定の受信
- `"service.gpu"`: クライアントモジュールの `gpu` モジュールの呼び出し（`client-full` ワールドだけ。`server-full` と `client-mod` では使えません）
- `"service.browser"`: `media-services`（`full::client::browser`）でブラウザを開く。入っている Edge か Chrome を、パッケージ専用のプロファイルで開きます。開けるのはウェブのページだけ（http・https・data・blob・about:blank）。絵は `GuiTexture` に、タブの音は PCM で受け取ります。`world = "client-full"` の `wasm.client` だけ。詳しくは `docs/yoake2/reports/FR-MEDIA1.md`。
- `"service.screen-share"`: 画面共有用に取ってあります（サービス番号 255〜259、まだつないでいません）。決まりは `service.browser` と同じです。
- `"service.http"`: `[http] hosts`（HTTPS のホストの一覧）と対になる `service.http` サービス権限の宣言です。`service.http` を宣言するなら `[http] hosts` が必要で、逆に `[http]` を書くなら `service.http` が必要です。HTTP 通信自体は（未実装）です[^dev-doc]
- `"service.permissions"`: サーバーモジュールからの `has_permission` の呼び出し（`wasm.server` が必須）
- `"service.storage"`: `storage_*` および `world_storage_*` / `config_read` / `config_write` の呼び出し

### `[config]` の書き方（`config-demo` のマニフェストより抜粋）

```toml
[wasm]
interface = "1.0"
client = "bin/wasm/config-demo.wasm"
world = "client-full"
permissions = ["service.config"]

[config.text]
type = "string"
side = "client"
default = "Config Demo HUD"
comment = "Text shown on the HUD banner."
label.en_us = "HUD Text"
label.ja_jp = "HUDテキスト"

[config.color]
type = "enum"
side = "client"
default = "gold"
values = ["gold", "aqua", "lime", "white"]
comment = "Colour preset for the HUD banner text."
label.en_us = "HUD Colour"
label.ja_jp = "HUD文字色"
```

さらに数値範囲（`min` / `max`）やリスト（`type = "list"`）、サーバーからクライアントへ同期する `side = "synced"` なども使えます。[^dev-doc]

```toml
[config.enabled]
type = "bool"
side = "server"
default = true
label.en_us = "Enabled"
label.ja_jp = "有効"

[config.radius]
type = "int"
side = "synced"
default = 8
min = 1
max = 16

[config.layers]
type = "list"
element = "enum"
side = "server"
default = ["surface"]
values = ["surface", "deep"]
```

- **使える型（`type`）**: `"bool"`, `"int"`, `"float"`, `"string"`, `"enum"`, `"list"`（`list` の場合は `element` に `"list"` 以外の要素型を指定します）。
- **使えるサイド（`side`）**: `"client"`（クライアント専用）、`"server"`（サーバー専用）、`"synced"`（サーバーで管理し、接続中のクライアントへ自動同期）。
- `[config]` を宣言した MOD は、ゲーム内の MOD 一覧画面の **Configure** ボタンから自動で設定画面が開けるようになります。

### `[permissions."<node>"]` の書き方（`permission-demo` のマニフェストより抜粋）

```toml
[wasm]
interface = "1.0"
server = "bin/wasm/permission-demo.wasm"
permissions = ["service.permissions"]

[permissions."secret.access"]
default_op_level = 2
```

- ノード名（`"secret.access"` など）はドット（`.`）区切りの英数字・`_`・`-` で書き、`default_op_level` には `0`〜`4` の OP レベルを指定します。
- `services::has_permission("secret.access", &uuid)` を呼ぶと、まずワールドフォルダ内の `<world>/permissions.toml` にプレイヤー別・グループ別の明示的な許可／拒否があるかを調べ、無ければ `default_op_level` 以上の OP 権限を持っているかで判定します。

---

## 次のステップ

MOD を改造していてエラーに出会ったときは、[5. よくあるエラーと直し方（`05-errors.md`）](./05-errors.md) を開いて原因と直し方を確認しましょう。より多くの書き方の例は [`docs/yoake-recipes.md`](../yoake-recipes.md) と [`docs/yoake-api-index.md`](../yoake-api-index.md) にもまとまっています。

[^hud-item]: 確認元: `crates/yoake-wasm-sdk/examples/hud-item/src/lib.rs`。
[^server-storage]: 確認元: `crates/yoake-wasm-sdk/examples/server-storage/src/lib.rs`。
[^storage]: 確認元: `docs/yoake-wasm-dev.md` の「Package config, permissions and storage」、`crates/yoake-wasm/src/host_services/storage.rs` の `DiskStorage::open_scoped_with_quota` / `import_store` / `import_directory` / `persist` / `atomic_write`。
[^storage-legacy]: 旧形式の取り込み: ストアを最初にバインドするとき、`World` / `Global` では旧 `store.bin` と、`World` では `<world>/yoake-wasm/<id>/<key>.bin`、`Global` では `<game>/config/yoake-wasm/<id>/<key>.bin` の旧エントリが、容量上限内で自動的にインポートされます。どちらの旧形式でもファイル名末尾の `.bin` はキーから取り除かれ、新しいストアに既にあるキーは置き換えません。`Client` はこれらの旧形式をインポートしません。旧ファイルと `store.bin` は復旧用バックアップとして残し、`migration-v2.done` マーカーで繰り返しのインポートを防ぎます。確認元: `docs/yoake-wasm-dev.md`。
[^manifest]: 確認元: `crates/horizon-mods/src/yak/manifest.rs`。
[^dev-doc]: 確認元: `docs/yoake-wasm-dev.md`。
