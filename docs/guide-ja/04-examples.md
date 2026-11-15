# 4. サンプル MOD 集を読む

`crates/yoake-wasm-sdk/examples/` には、目的別のサンプル MOD が入っています。
自分が作りたい機能に近いサンプルを開いて、`yak.toml` と `src/lib.rs` を真似するのが一番の近道です。

---

## 4-1. サンプル一覧（何を見せているか・使うフックと service）

| サンプル名 | WIT ワールド（MOD の種類） | 何を見せているか | 主なフック（`hook::*`） | 主なサービス・ホスト呼び出し |
|---|---|---|---|---|
| `hello`（および `hello-std`） | `client-mod` | 最小のクライアント MOD。ティック数を数えて HUD に背景矩形と文字を描画し、設定ボタン名を返す（`hello` は `#![no_std]`、`hello-std` は `std` 使用） | `TICK`, `HUD`, `SETTINGS`（`hello-std` は `TICK`, `KEY`, `HUD`） | `yoake_wasm_sdk::log`, `hud::rect`, `hud::text` |
| `hud-item` | `client-full` | 毎フレーム HUD に半透明パネル・`minecraft:diamond` のアイテムアイコン・ラベル文字を描画する | `HUD` | なし（`et::HudOutcome` に `Rect`, `Item`, `Text` を詰めて返す） |
| `server-storage` | `server-full` | `/visits` コマンドを追加し、プレイヤー UUID ごとの訪問回数をワールドストレージに保存して返答する | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::storage_get`, `services::storage_put`, `services::tell` |
| `config-demo` | `client-full` | `yak.toml` の `[config]` で定義した文字と色を読み取り、設定変更時に `config_changed` で HUD 表示を更新する | `HUD`, `CONFIG_CHANGED`（＋ `init`） | `services::config_get` |
| `permission-demo` | `server-full` | `/secret` コマンドを追加し、`has_permission` で `"secret.access"` 権限ノードの有無を判定して返答を変える | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::has_permission`, `services::tell` |
| `full-demo` | `client-full` / `server-full` | クライアント（HUD・ホットバー非表示・ツールチップ追加・FOV 変更）、サーバー（`/wasmping`・`/wasmsvc` で各種サービス巡回）、別 MOD 連携（`peer`）の総合デモ | client: `HUD`, `GUI_LAYER_PRE`, `TOOLTIP`, `COMPUTE_FOV` / server: `REGISTER_COMMANDS`, `PERFORM_COMMAND`, `SERVER_TICK` / peer: `handle_call` | `tell`, `give_items`, `break_block_as`, `edit_blocks`, `run_command`, `mod_call` ほか多数 |
| `full-services` | `client-full` / `server-full` | 設定・ストレージ（通常および分割 blob）・権限チェックの一連の操作と、権限未宣言時に `"permission denied: ..."` で拒否される動作の検証 | `CONFIG_CHANGED`（＋ client `init` / server `handle_call`） | `config_get`, `config_set`, `storage_get`, `storage_put`, `storage_delete`, `storage_blob_begin/write/commit/abort/read`, `has_permission` |
| `mod-call-pair` | `server-full` | `caller` の `/ping <text>` から `responder` の `handle_call` を呼び、2 つの MOD で UTF-8 テキストをやり取りする | `REGISTER_COMMANDS`, `PERFORM_COMMAND` / `handle_call` | `services::mod_call`, `services::tell` |
| `chest-count` | `server-full` | `/chestcount <x> <y> <z>` で指定した座標のコンテナを読み、アイテム ID ごとの合計を返答する | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::container_slots`, `services::tell` |
| `item-trade` | `server-full` | `/trade` で丸石をちょうど 16 個、全部取れるときだけ取り出し（足りなければ何も取らない）、ダイヤモンド 1 個を与える | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::inventory_apply`（`Give`）, `ext::take`, `ext::take_exact`, `services::tell` |
| `player-vitals` | `server-full` | `/vitals` でプレイヤーの体力と `minecraft:story/root` 実績の `known` / `done` を返答する | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::advancement_progress`, `services::attribute_get`, `services::tell` |

---

## 4-2. 各サンプルのポイントとコード抜粋

### ① `hello` / `hud-item`（HUD に文字やアイテムを描く）

- 19 個の基本フックだけで足りる場合は `hello` のように `yoake_wasm_sdk::Mod` と `export_mod!` を使います。
- アイテムアイコン（`HudCommand::Item`）やツールチップ変更などフルカタログの機能を使う場合は、`hud-item` のように `yak.toml` に `world = "client-full"` を書き、`ClientMod` と `export_client_full!` を使います。

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

サーバー側で動く MOD は `yak.toml` の `[wasm]` に `server = "bin/wasm/..."` を指定します（サーバーモジュールの WIT ワールドは常に `server-full` です）。
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

- 保存先（`StorageScope`）は、サーバー側ではセーブデータのワールドごとの `World`（`<world>/yoake-data/<mod-id>/store.bin`）と全ワールド共通の `Global`（`<game>/yoake-data/<mod-id>/global/store.bin`）、クライアント側では `Client`（`<game>/yoake-data/<mod-id>/client/store.bin`）を選べます。現行実装では、各スコープの全キーを 1 つの `store.bin` に同期保存します。[^storage]

> 保存ファイル（`store.bin`）の形は今後の版で変わる可能性があります。中身を直接書き換えたり、
> 形に頼った道具を作ったりせず、必ず `storage_get` / `storage_put` を使ってください。

### `mod-call-pair`（別の MOD を呼び出す）

`caller` と `responder` は別々の `.yak` パッケージなので、両方をインストールします。`caller` は `/ping <text>` の文字列をバイト列にして `services::mod_call("echo", ..., "mod-call-responder")` で送り、`responder` は `handle_call` で `pong` の返事を返します。呼び出し先には `yak.toml` のパッケージ ID を使います。`yak.toml` に呼び出し先を書く欄はなく、追加権限もありません。`responder` をインストールして有効にしていない場合、プレイヤーには `no package ... with a server module` というエラーが表示されます。どちら側も、受け取ったバイト列を UTF-8 へ変換できなければ、panic せずにエラーとして扱います。

### `chest-count`（コンテナのアイテムを数える）

`/chestcount <x> <y> <z>` は、コマンドを実行したプレイヤーと同じディメンションの指定座標を `services::container_slots` で読みます。各スロットの `Option<types::ItemStack>` から中身だけを取り出し、アイテム ID ごとに `count` を合計します。ラージチェストは両半分のスロットが返ります。対象はコマンドを実行したプレイヤーが今いるディメンションのコンテナだけです。チェスト以外のコンテナにも使えますが、コンテナが無い座標や読み込まれていないチャンクではエラーになり、コンポスターもコンテナとしては扱われません。

### `item-trade`（アイテムを交換する）

`/trade` は、先に `services::inventory_apply` の `Give` で `minecraft:diamond` を 1 個与えます（メインスロットだけが対象で、全量入るか何も入らないかのどちらかです）。入らなければ「インベントリがいっぱい」と知らせて終了し、何も変わりません。入ったら `ext::take` で `minecraft:cobblestone` を 16 個ちょうど取り出します。これも全量または無しの操作で、オフハンドも対象です。取り出しに失敗した（丸石が足りない）ときは、いま与えたダイヤモンドを `ext::take_exact` で取り戻して取引を取り消します。順序を「与えてから取る」にしているので、入力を返す処理は要りません。`ext::take` は名前やタグなどのコンポーネントが付いた丸石も取りますが、`ext::take_exact` はアイテム・個数・コンポーネントが完全に一致するスタックだけを取ります。

### `player-vitals`（体力と実績の進捗を読む）

`/vitals` はコマンドフックの `player.health` から現在の体力を、`services::attribute_get("minecraft:max_health", ...)` から最大体力を読み、`services::advancement_progress` で `minecraft:story/root` の状態を読みます。返ってくる `types::AdvancementState` の `known` は実績を認識しているか、`done` は完了しているかを表します。実績や最大体力の読み取りに失敗した場合も、取得できている体力は返答します。

---

## 4-3. `yak.toml` での `[wasm] permissions`・`[config]`・`[permissions."node"]` の書き方

MOD が特別な機能（設定の読み書き、ストレージ保存、権限チェック、サーバーコマンド実行など）を使うときは、`yak.toml` に宣言を書きます。

### `[wasm] permissions` の種類

指定可能な権限（capability）は次の 5 つです。[^manifest]

- `"service.config"`: `config_get` / `config_set` / `config_dir` の呼び出しと、同期設定の受信
- `"service.storage"`: `storage_*` および `world_storage_*` / `config_read` / `config_write` の呼び出し
- `"service.permissions"`: サーバーモジュールからの `has_permission` の呼び出し（`wasm.server` が必須）
- `"service.commands"`: サーバーモジュールからの `run_command` の呼び出し（`wasm.server` が必須）
- `"service.http"`: `[http] hosts` と対になる HTTP 権限宣言（ただし HTTP 通信自体は現時点では未接続）[^dev-doc]

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

### `[permissions."node"]` の書き方（`permission-demo` のマニフェストより抜粋）

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

MOD を改造していてエラーに出会ったときは、[5. よくあるエラーと直し方（`05-errors.md`）](./05-errors.md) を開いて原因と直し方を確認しましょう。

[^hud-item]: 確認元: `crates/yoake-wasm-sdk/examples/hud-item/src/lib.rs`。
[^server-storage]: 確認元: `crates/yoake-wasm-sdk/examples/server-storage/src/lib.rs`。
[^storage]: 確認元: `crates/yoake-wasm/src/host_services/storage.rs` の `DiskStorage::directory` / `open_scoped_with_quota` / `persist`。
[^manifest]: 確認元: `crates/horizon-mods/src/yak/manifest.rs`。
[^dev-doc]: 確認元: `docs/yoake-wasm-dev.md`。
