# 14. カスタム Mob を作る（F4M）

F4M を使うと、バニラのモブをコピーせずに**オリジナルの Mob（モデル・アニメーション・当たり判定・AI・乗り物座席）**を `.yak` パッケージで作れます。Blockbench の `.bbmodel` や Bedrock 形式の `.geo.json` / `.animation.json` をそのまま読み込めます。

> [!NOTE]
> モブの見た目やデータ定義は `content/` に JSON と画像で置き、AI やイベントへの反応は `server-full`（必要に応じて `client-full`）の Wasm モジュールで書きます。`[wasm] permissions` に `"service.mobs"` を宣言してください。

---

## 14-1. `custom-mob` テンプレートですぐ始める

作業フォルダで次を実行すると、動くカスタム Mob のひな形が作られます。

```sh
yoake new custom-mob my-mob
cd my-mob
yoake build
```

生成されるフォルダ構成は次のとおりです。

```text
my-mob/
  yak.toml
  Cargo.toml
  src/lib.rs                                               <- ScriptGoal "pounce" の server-full コード
  content/
    horizon-mod.toml                                       <- api = "0.2", capabilities = ["registries"]
    data/my_mob/horizon/entity_type/starter_mob.json       <- Mob の定義（"template": "yoake:mob"）
    assets/my_mob/
      geo/starter_mob.geo.json                             <- Bedrock ジオメトリ
      animations/starter_mob.animation.json                <- アニメーション定義
      controllers/starter_mob.json                         <- アニメーションコントローラー
      textures/entity/starter_mob.png                      <- 64x32 テクスチャ
```

- `yoake new` の `<name>` にハイフン（`-`）を使うと、コンテンツの名前空間では自動でアンダースコア（`_`）に置き換わります（例: `my-mob` → `id = "my_mob"`、エンティティ ID は `my_mob:starter_mob`）。
- テンプレートの `yak.toml` には次の設定が入ります。

```toml
[side]
client = true
server = true
required_on_join = false

[wasm]
interface = "1.7"
server = "bin/wasm/my-mob.wasm"
permissions = ["service.mobs"]
```

---

## 14-2. モブ定義ファイル（`entity_type/*.json`）の主な項目

`content/data/<ns>/horizon/entity_type/<name>.json` に `"template": "yoake:mob"` を書いて定義します（一部抜粋。完全な例は [`tools/fixtures/cinder_fox/content/data/sample_mobs/horizon/entity_type/cinder_fox.json`](../../tools/fixtures/cinder_fox/content/data/sample_mobs/horizon/entity_type/cinder_fox.json) にあります）。

```json
{
  "template": "yoake:mob",
  "format": 1,
  "category": "creature",
  "size": { "width": 0.6, "height": 0.7, "eye_height": 0.5 },
  "hitboxes": [
    { "name": "body", "size": [0.6, 0.7] },
    { "name": "head", "size": [0.4, 0.4], "bone": "head", "damage_multiplier": 1.5 }
  ],
  "attributes": {
    "max_health": 14,
    "movement_speed": 0.3
  },
  "model": {
    "geo": "sample_mobs:geo/cinder_fox",
    "textures": { "default": "sample_mobs:entity/cinder_fox" },
    "animations": ["sample_mobs:animations/cinder_fox"],
    "controllers": ["sample_mobs:controllers/cinder_fox"],
    "server_events": ["pounce"],
    "shadow": { "radius": 0.4, "strength": 0.8 }
  },
  "synced": {
    "state": { "type": "enum", "values": ["idle", "sit", "pounce"], "default": "idle", "persistent": true }
  },
  "spawn_egg": { "primary": "0xD2691E", "secondary": "0xFFF0A0" },
  "proxy": "minecraft:fox"
}
```

- **`size` と `hitboxes`**: `size` がワールドの衝突ボックス（1 つだけ）です。`hitboxes` に複数の攻撃・照準ボックスを足せます。`bone` を書くとモデルのボーンに追従し、`damage_multiplier` で部位ごとのダメージ倍率を変えられます。
- **`synced`**: サーバーからクライアントへ自動同期する値（`bool`, `i32`, `f32`, `string`, `vec3`, `enum`、1 型最大 32 個・合計 256 バイト）。コントローラーの式やクライアント MOD から読めます。
- **`spawn_egg`**: `{ "primary": "0x...", "secondary": "0x..." }` を書くと `<ns>:<name>_spawn_egg` アイテムが自動生成されます。
- **`proxy`**: バニラクライアントが参加したときに代わりに表示するバニラモブ ID（例: `"minecraft:fox"`）。省略したカスタム Mob があるワールドは、Horizon クライアント専用になります。

---

## 14-3. モデルとアニメーション

- **モデル（`model.geo`）**: `assets/<ns>/geo/<name>.geo.json`（Bedrock 1.8 / 1.12+）または `assets/<ns>/geo/<name>.bbmodel`（Blockbench プロジェクト直読み）に対応しています。
- **アニメーション（`model.animations`）**: `assets/<ns>/animations/<name>.animation.json` を読み込みます。`server_events` にアニメ名を書くと、そのタイムラインイベント（例: `"hit"`）がサーバーの `MOB_ANIM_EVENT` フックでも発火します。
- **発光テクスチャ**: メインテクスチャの隣に `<name>_glowmask.png` を置くか、`model.layers` に `"mode": "emissive"` を書くと暗闇で光るレイヤーを描けます。
- **コントローラー（`model.controllers`）**: `assets/<ns>/controllers/<name>.json` でステート遷移や単発トリガーを定義します（[`cinder_fox.json`](../../tools/fixtures/cinder_fox/content/assets/sample_mobs/controllers/cinder_fox.json) より抜粋）。

```json
{
  "controllers": {
    "locomotion": {
      "initial": "idle",
      "states": {
        "idle": {
          "animations": ["idle"],
          "transitions": [{ "to": "walk", "when": "q.ground_speed > 0.05" }],
          "blend": 0.15
        },
        "walk": {
          "animations": [{ "name": "walk", "speed": "q.ground_speed * 4" }],
          "transitions": [{ "to": "idle", "when": "q.ground_speed < 0.03" }]
        }
      }
    },
    "action": {
      "weight": 1,
      "triggers": { "pounce": "pounce" }
    }
  }
}
```

---

## 14-4. `ScriptGoal` と `service.mobs` で AI を書く

### JSON 側の宣言（`ai`）

データゴール（`swim`, `wander`, `look_at_player`, `random_look`, `follow_owner`, `sit`, `nearest_target` など）と並べて、`"type": "script"` のゴールを書けます。さらに `memories`・`sensors`・`activities`・`schedule`（brain オプション）も使えます。

```json
"ai": {
  "navigator": "ground",
  "memories": { "pounced": { "type": "bool" } },
  "goals": [
    { "type": "swim", "priority": 0 },
    { "type": "script", "id": "pounce", "priority": 2 },
    { "type": "wander", "priority": 6, "speed": 1.0 }
  ],
  "script_goals": [
    { "id": "pounce", "priority": 2, "flags": ["move", "look"], "interval": 10 }
  ]
}
```

### Rust 側の実装（`server-full` + `service.mobs`）

`yoake_wasm_sdk::full::server::mobs` でモブの状態取得・移動・攻撃・アニメ再生・メモリ読み書き・同期値更新を行います。`custom-mob` テンプレートの抜粋です。

```rust
use yoake_wasm_sdk::full::hook;
use yoake_wasm_sdk::full::server::{mobs, ServerMod};

impl ServerMod for Starter {
    fn subscriptions() -> &'static [&'static str] {
        &[
            hook::MOB_GOAL_CAN_USE,
            hook::MOB_GOAL_CAN_CONTINUE,
            hook::MOB_GOAL_START,
            hook::MOB_GOAL_TICK,
            hook::MOB_GOAL_STOP,
        ]
    }

    fn mob_goal_can_use(&mut self, mob: u32, goal: String) -> bool {
        if goal != "pounce" {
            return false;
        }
        let Ok(Some(info)) = mobs::info(mob) else { return false };
        info.type_id == "my_mob:starter_mob" && info.on_ground && info.target_visible
    }

    fn mob_goal_start(&mut self, mob: u32, goal: String) {
        if goal == "pounce" {
            let _ = mobs::navigate_stop(mob);
            let _ = mobs::trigger_anim(mob, "", "pounce", 1.0);
        }
    }
    // mob_goal_can_continue / mob_goal_tick / mob_goal_stop も同様に実装します
}
```

---

## 14-5. 座席（`seats`）で乗り物・複数人乗りを作る

`entity_type/*.json` に `seats` 配列（最大 8 席）を書くと、カスタム Mob にプレイヤーや他のモブを乗せられます。

```json
"seats": [
  { "offset": [0.0, 0.8, 0.0], "controls": "player", "yaw_lock": true },
  { "locator": "mouth", "controls": "none", "allowed_types": ["minecraft:chicken"] }
]
```

- `offset`: モブ原点からの座標 `[x, y, z]`。代わりにモデルのロケーター名を指定する `"locator"` も使えます。
- `controls`: `"player"`（運転席）または `"none"`（同乗席）。
- `allowed_types`: 乗れるエンティティ ID の一覧（省略時は制限なし）。
- サーバー側コードからは `mobs::seat_set(mob_id, seat_index, Some(rider_id))` でモブを座席に乗せ、`None` で降ろせます。

---

## 14-6. サンプル（Cinder Fox シリーズ）と開発コマンド

リポジトリには、データ定義（[`tools/fixtures/cinder_fox/`](../../tools/fixtures/cinder_fox/README.md)）と連動する 3 つの Wasm サンプルがあります。

- [`crates/yoake-wasm-sdk/examples/fox_ai/`](../../crates/yoake-wasm-sdk/examples/fox_ai/src/lib.rs) — サーバー側: `ScriptGoal` `"pounce"`、アニメイベント `"hit"` での飛びかかり攻撃、被弾時に同期値 `warmth` を更新。
- [`crates/yoake-wasm-sdk/examples/fox_texture/`](../../crates/yoake-wasm-sdk/examples/fox_texture/src/lib.rs) — クライアント側（`client-full` + `service.mobs`）: `SYNCED_CHANGED_CLIENT` で `warmth` を受け取り、`mob_texture_create` / `mob_texture_write` / `mob_texture_bind` で実行時に肌の色を塗り替え、`mob_layer` で頭上に立方体とリンゴを表示。
- [`crates/yoake-wasm-sdk/examples/seat_fox/`](../../crates/yoake-wasm-sdk/examples/seat_fox/src/lib.rs) — サーバー側: `mobs::seat_set` で近くのニワトリを自動で座席 1 に乗せる例。
- （参考）フルアクセス MOD 版の `fox_ai`: [`crates/yoake-guest-sdk/examples/fox_ai_full.rs`](../../crates/yoake-guest-sdk/examples/fox_ai_full.rs)

ゲーム内デバッグ・開発用コマンド:

- `/yoake mobdebug [on|off] [hitboxes] [bones] [anim] [ai]`（または `F3+M`）— ヒットボックス、ボーン、アニメ状態、AI 経路を可視化します。
- `/yoake mobs` — 型ごとの描画・経路コストを表示します。
- `/yoake mob reload [型]` — 開発者モード（`YOAKE_WASM_UNSIGNED=1`）中に JSON やモデルを再読み込みします。

くわしい仕様は [`docs/yoake2/MOD-GUIDE.ja.md` 第 17 章](../yoake2/MOD-GUIDE.ja.md#17-カスタム-mob-と-brain-オプション-f4m-4b) と [`docs/mods.md`](../mods.md#custom-mobs-f4m) を参照してください。
