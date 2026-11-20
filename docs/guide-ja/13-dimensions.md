# 13. ディメンション・レベル・チャンクジェネレーター

Yoake は Minecraft 26.3 のデータパックと同じ仕組みでディメンション（エンジン内部では **レベル / level** と呼びます）を管理しています。データパックの JSON だけで新しいディメンションを増やせるほか、`server-full` の WebAssembly MOD から実行中にレベルを作ったり消したり、チャンクジェネレーター（地形を作る処理）を Rust で書いたりできます。

> [!NOTE]
> この章の対象は、データパックの JSON と **サンドボックス化された WebAssembly MOD（`server-full`）** です。MOD を入れていないときは、バニラの 3 ディメンション（`minecraft:overworld`、`minecraft:the_nether`、`minecraft:the_end`）の動きは一切変わりません。

---

## 13-1. データパックの JSON だけでディメンションを追加する

コードを書かずにディメンションを追加するには、ワールドのデータパック（`<ワールド>/datapacks/<パック名>/`）に **`dimension_type`（ディメンション型）** と **`dimension`（レベルステム）** の JSON を置きます。

> [!IMPORTANT]
> 現在のところ、コンテンツ MOD（`.hmod`）や `.yak` 内の `content/` からは `dimension` や `dimension_type` を宣言できません。JSON のみで追加する場合はワールドの `datapacks/` を使うか、後述する WASM MOD の `register_type` / `register_level` を使ってください。

### フォルダ構成と保存先

たとえば `mypack:moon` というディメンションを追加する場合、データパック内のファイルは次の場所に置きます。

```text
<ワールド>/datapacks/moon/
  pack.mcmeta
  data/
    mypack/
      dimension_type/
        moon_type.json       <- 型 ID: mypack:moon_type
      dimension/
        moon.json            <- レベル ID: mypack:moon
```

このディメンションのチャンク・エンティティ・POI（Point of Interest）などのセーブデータは、バニラ 26.3 と同じくワールドフォルダ内の以下に保存されます。

```text
<ワールド>/dimensions/<namespace>/<path>/
  region/
  entities/
  poi/
```

`mypack:moon` なら `<ワールド>/dimensions/mypack/moon/` になります。

### `dimension_type`（ディメンション型）の例と各属性の働き

`dimension_type` は空・光・天候・ベッド・溶岩・高さなど、そのディメンションのルールを決めます（[テスト用フィクスチャ](../../crates/horizon-client/src/game/level_registry_tests.rs)の抜粋をもとにした例です）。

```json
{
  "min_y": 0,
  "height": 128,
  "logical_height": 128,
  "has_skylight": true,
  "has_ceiling": false,
  "ambient_light": 0.3,
  "coordinate_scale": 1.0,
  "default_clock": "minecraft:overworld",
  "skybox": "overworld",
  "cardinal_light": "default",
  "timelines": "#minecraft:in_overworld",
  "attributes": {
    "minecraft:visual/sky_color": "#336699",
    "minecraft:visual/fog_color": "#223344"
  }
}
```

主なフィールドと属性（`attributes`）は次のように効きます。

| 分類 | フィールド / 属性 | 変わるもの |
|---|---|---|
| 高さ | `min_y`、`height`、`logical_height` | 建築できる高さとチャンクセクション数。モブの湧き上限は `min_y + logical_height - 1` になります。 |
| 空・昼夜 | `skybox`、`has_skylight`、`has_ceiling`、`has_fixed_time`、`timelines`、`default_clock` | `skybox` は `"overworld"` / `"end"` / `"none"` を選びます。`skybox` が `"overworld"` で、固定時刻でない（`has_fixed_time: false`）か `timelines` を指定しているときに昼夜サイクル（`day_cycle`）が動きます。 |
| 霧・空の色 | `attributes` の `minecraft:visual/sky_color`、`fog_color`、`fog_start_distance`、`fog_end_distance`、`cloud_fog_end_distance`、`cloud_height` など | 空と霧の色・距離・雲の高さを決めます。`fog_end_distance` があると環境霧の距離が有効になります。 |
| 光 | `ambient_light`、`cardinal_light` | `ambient_light` は全体の最低限の明るさです。`cardinal_light` を `"nether"` にすると、ネザー風のブロック陰影とライトマップになります。 |
| 天候 | `skybox` + `has_skylight` + `has_ceiling` | `skybox == "overworld"` かつ `has_skylight == true` かつ `has_ceiling == false` のときだけ、雨と雷が降ります。 |
| ベッド・アンカー | `attributes` の `minecraft:gameplay/bed_rule`、`straw_bed_rule`、`respawn_anchor_works` | `bed_rule` で `can_sleep`（`"always"` / `"when_dark"` / `"never"`）、`can_set_spawn`（`"always"` / `"when_dark"` / `"never"`）、`destroy_on_use` を設定します。`respawn_anchor_works` が `true` ならリスポーンアンカーが使えます（古い `respawn_anchor_works` 直書きも読めます）。 |
| 溶岩・水 | `attributes` の `minecraft:gameplay/fast_lava`、`water_evaporates` | `fast_lava` で溶岩の流れが速くなり、`water_evaporates` で水が蒸発しバケツから置けなくなります（古い `"ultrawarm": true` は両方を `true` にします）。 |
| モブ・襲撃 | `attributes` の `minecraft:gameplay/piglins_zombify`、`snow_golem_melts`、`can_start_raid`、`monster_spawn_light_level`、`monster_spawn_block_light_limit`、`infiniburn` | ピグリンのゾンビ化、スノウゴーレムの融解、襲撃の発生、モンスターの湧き明るさ条件（`0..=15` の定数または `minecraft:uniform`）、消えない火のブロックタグ（`#` で始まる）を決めます。 |
| 座標倍率 | `coordinate_scale` | ポータルや `/execute in` で移動するときの座標の倍率です（オーバーワールドは `1.0`、ネザーは `8.0`）。 |

> [!NOTE]
> **海面（`sea_level`）は `dimension_type` ではなくジェネレーターで決まります。**
> `minecraft:noise` の場合は参照する `noise_settings` の `sea_level`、`minecraft:flat` は `-63`、`minecraft:debug` は `63` が使われます。海面を持たないジェネレーター（`yoake:mod` など）では、`has_ceiling` が `true` なら `32`、`false` なら `63` にフォールバックします。

### `dimension`（レベルステム）の例

`dimension/*.json` は、どの `dimension_type` とどの `generator` を使うかを結びつけます。

```json
{
  "type": "mypack:moon_type",
  "generator": {
    "type": "minecraft:noise",
    "settings": "minecraft:overworld",
    "biome_source": {
      "type": "minecraft:fixed",
      "biome": "minecraft:plains"
    }
  }
}
```

`"type"` には `"mypack:moon_type"` のような登録済み ID のほかに、ディメンション型 JSON をそのままインラインで書くこともできます。

### 検証エラー（バリデーション）

データパックや MOD が不正な型・ステムを渡した場合、エンジンはエラー（`LevelProblem` / `LevelError`）をログに出してそのレベルだけを登録から外します。バニラの 3 ディメンションが巻き添えで壊れることはありません（バニラ ID に不正なステムを被せた場合も、エラーを報告したうえでバニラ設定のまま動きます）。

よく見かける検証エラーは次のとおりです。

- **ID の形式エラー（`LevelError::BadId`）**: レベル ID はそのままフォルダ名になるため、`namespace:path` 形式で、名前空間は `[a-z0-9_.-]`（`.` や `..` は不可）、パスは `[a-z0-9_./-]`（空要素・`.`・`..`・大文字・空白は不可）である必要があります。
- **`min_y` と `height` の範囲エラー**: `min_y` と `height` はどちらも必須で、**16 の倍数**でなければなりません。`min_y >= -2032`、`height >= 16`、`min_y + height <= 2032`、`0 <= logical_height <= height` を満たす必要があります。
- **値の範囲・型エラー**: `coordinate_scale` は `1e-5` 以上 `3e7` 以下の有限数、`ambient_light` は有限数、`monster_spawn_block_light_limit` と `monster_spawn_light_level` は `0..=15`、`infiniburn` は `#` で始まるブロックタグである必要があります。
- **未定義の型・不正なジェネレーター**: `"type"` が存在しない ID だと `unknown dimension type`、`generator.type` が未対応だと `unsupported generator type` になります。`yoake:mod` で `"id"` が抜けていたり `:` を含まない場合も拒否されます。

---

## 13-2. ジェネレーターとバイオームソース、レベルごとのシード値

### 使える 4 種類のジェネレーター

レベルステムの `generator.type` には次の 4 種類を指定できます。

1. **`minecraft:noise`**: 通常のノイズ地形です。`"settings"` にノイズ設定の ID（例: `"minecraft:overworld"`）またはインライン JSON オブジェクトを、`"biome_source"` にバイオーム配置を指定します。
2. **`minecraft:flat`**: スーパーフラット地形です。`"settings"` オブジェクトの中に `"biome"` や `"layers"` を書きます（バニラと同じ形式）。オーバーワールド以外のディメンションでも使えます。

```json
{
  "type": "minecraft:overworld",
  "generator": {
    "type": "minecraft:flat",
    "settings": {
      "biome": "minecraft:plains",
      "features": false,
      "lakes": false,
      "layers": [
        { "block": "minecraft:bedrock", "height": 1 },
        { "block": "minecraft:stone", "height": 2 },
        { "block": "minecraft:dirt", "height": 1 },
        { "block": "minecraft:grass_block", "height": 1 }
      ]
    }
  }
}
```

3. **`minecraft:debug`**: デバッグワールド（`y = 60` のバリア床と全ブロック状態の一覧）です。追加設定は要りません。

```json
{
  "type": "minecraft:overworld",
  "generator": { "type": "minecraft:debug" }
}
```

4. **`yoake:mod`**: Yoake 独自の拡張で、MOD が登録したチャンクジェネレーターを呼び出します。`"id"` に登録済みのジェネレーター ID、`"settings"` に任意の JSON オブジェクト（省略時は `{}`）を書きます。詳しくは 13-4 節で説明します。

### バイオームソース（`biome_source`）

`minecraft:noise` の `"biome_source"` では、次の形式が使えます。

- **`minecraft:fixed`**: 全体を 1 つのバイオームにします。
  ```json
  { "type": "minecraft:fixed", "biome": "minecraft:desert" }
  ```
- **`minecraft:checkerboard`**: バイオームのリスト（または `#` タグ）を市松模様に並べます。`scale` は `0..=62`（省略時は `2`）で、1 マスの幅は `1 << (scale + 2)` クォート（`scale: 0` なら 16 ブロック＝ 1 チャンク単位）になります。
  ```json
  {
    "type": "minecraft:checkerboard",
    "biomes": ["minecraft:plains", "minecraft:desert"],
    "scale": 0
  }
  ```
- **`minecraft:multi_noise`**: 気候パラメータ（`temperature`、`humidity`、`continentalness`、`erosion`、`depth`、`weirdness`、`offset`）のリストを `"biomes"` に並べるか、`"preset"`（`"minecraft:overworld"` / `"minecraft:nether"` など）を指定します。
- **`minecraft:the_end`**: ジ・エンド用の 5 つのバニラバイオーム配置を使います。
- **`yoake:grid`**（Yoake 拡張）: 指定した `cell_size`（4 の倍数）と `origin`（4 の倍数の `[x, z]`）、`width` / `depth`（`1..=256`）の格子でバイオームを直指定し、範囲外は `outside` を使う高速なテーブル配置です。

### レベルごとのシード値（`"seed"`）

バニラではすべてのディメンションがワールド共通のシード値を使いますが、Yoake ではレベルステムに **`"seed"`**（整数または数字の文字列、例: `"seed": -77` や `"seed": "123"`）を書くことで、そのレベル専用のシード値を持たせられます。省略するとワールドのシード値が使われます。

`create_level` で作ったレベルでは、あとから `regenerate_level(id, Some(new_seed))` でシード値を差し替えることもできます。

---

## 13-3. ディメンション間の移動と再接続ルール

### `/execute in` と `/tp`

登録されているすべてのレベルは、コマンドから指定して移動できます。

- `/execute in mypack:moon run tp @s 0 80 0`
- `/execute in mypack:moon run tp @s ~ ~ ~`

`/execute in <レベル ID>` は、コマンド実行元（ソース）の座標を **`coordinate_scale(移動元) / coordinate_scale(移動先)`** でスケーリングします。そのため、上の 2 つ目の例のように相対座標 `~ ~ ~` で `/tp` すると座標倍率が反映され、1 つ目の例のように絶対座標 `0 80 0` を渡すとその座標へ直接移動します。`/tp <対象> <別次元のエンティティ>` も相手のレベルへ移動します。未登録のレベル ID を指定するとエラー（`argument.dimension.invalid`）になります。

### ポータルの座標倍率

`Game::travel_scale(from, to)` は `coordinate_scale(from) / coordinate_scale(to)` を計算します。オーバーワールド（`1.0`）とネザー（`8.0`）のあいだでは `1/8` と `8` になります。ただし、**ネザーポータルの自動生成・リンク先や、エンドポータル／エンドゲートウェイの行き先はバニラ 3 ディメンションに固定**されているため、独自ディメンションで独自のポータル移動を作りたい場合は MOD 側のテレポート処理を使います。

### 接続中にレベルを作ったときの「再接続ルール」

サーバーの起動後、プレイヤーがすでに接続している最中に `create_level` で新しいレベルを作ったときは、Minecraft の通信プロトコル上の制約があります。

`minecraft:dimension_type` レジストリの一覧は、**プレイヤーがサーバーに参加するログイン／設定フェーズでしか送れません**。そのため、次のように動作が分かれます。

- **新規参加プレイヤー**: 作成直後から、追加されたレベルとその型がログイン時に送られるため、そのまま入れます。
- **すでに接続中のプレイヤー（既存の型を使う場合）**: 作成したレベルが `"minecraft:overworld"` や、接続前からデータパック／`init` で登録済みだった型を使うなら、**再接続なしでそのまま入れます**（クライアントはリスポーン時に未リストの実行時レベルを最大 64 個まで自動で受け入れます）。
- **すでに接続中のプレイヤー（新しい型を使う場合）**: インライン定義など、そのプレイヤーの接続後に初めて登場した `dimension_type` を使うレベルへ移動しようとすると、サーバーは移動を拒否し、チャットに **再接続が必要である旨のメッセージ（`reconnect to enter it`）** を出します。一度切断して入り直すと入れるようになります。

---

## 13-4. WebAssembly MOD からレベルとチャンクジェネレーターを作る

`server-full` ワールドの WASM MOD は、`yoake_wasm_sdk::full::server::ext` と `yoake_wasm_sdk::full::generator` を使って、コードからディメンション型・レベル・チャンクジェネレーターを追加・操作できます。

### `yak.toml` の設定

サーバー側で動かす `server-full` モジュールとして設定します（`crates/yoake-wasm-sdk/examples/level-generator/yak.toml` より）。

```toml
format = 1
id = "level-generator"
version = "0.1.0"
name = "Level Generator"
authors = ["Yoake"]
description = "Registers the level levelgen:terraces: a dimension type of its own (64 high, a blue sky) and a chunk generator written in WebAssembly."

[yoake]
version = "*"

[side]
client = false
server = true
required_on_join = false

[wasm]
interface = "1.0"
world = "server-full"
server = "bin/wasm/level-generator-server.wasm"
```

### `init` での登録（型・レベル・ジェネレーター）

ワールドが開く前から存在してほしい型・レベル・ジェネレーターは、`ServerMod::init` の中で登録します。

- `ext::register_type(id, type_json)`: ディメンション型を登録します。
- `ext::register_generator(id)`: このパッケージが `"type": "yoake:mod", "id": id` のチャンク生成を担当することを宣言します。
- `ext::register_level(id, stem_json)`: レベルを登録します（保存先は `<ワールド>/dimensions/<ns>/<path>/`）。

`init` はワールドを開く前のステージング領域に登録するため、プレイヤーが前回その MOD レベルでログアウトしていても、次回ワールドを開いたときにそのままそのレベルへ復帰できます。なお、ワールドが最初のティックを進めたあと（`freeze()` 後）に `register_level` や `register_generator` をフックから呼ぶと拒否されます。

### 実行中のレベル管理（永続・一時）

ゲームの進行に合わせてサーバーフックの中からレベルを操作するには、以下の関数を使います。

| 関数 | できること・ルール |
|---|---|
| `ext::create_level(id, stem_json, persistent)` | 実行中にレベルを作ります（同時に最大 **64 個**まで）。`persistent == true` なら `dimensions/<ns>/<path>/` に保存し、`<ワールド>/data/yoake_levels.json` に記録して次回起動時にも復元します。`persistent == false`（一時レベル）なら `<ワールド>/yoake_temp/<ns>/<path>/` に保存し、削除時または次回ワールド起動時に消去します。 |
| `ext::delete_level(id)` | `create_level` で作ったレベルと保存フォルダを削除します。誰かがそのレベルにいる／移動中（`Occupied`）、バニラの 3 レベル（`Vanilla`）、データパックや `register_level` 由来のレベル（`NotCreated`）は拒否されます。 |
| `ext::unload_level(id)` | バニラ以外のレベルをセーブして閉じ（チャンクと region ファイルを解放）、登録状態だけは残します。次に誰かが入るとフォルダから開き直します。実際の解放は **次のサーバーティックの先頭** で実行されます。誰かがいる・向かっているレベルは拒否されます。 |
| `ext::regenerate_level(id, seed)` | バニラ以外のレベルを閉じ、`region`・`entities`・`poi` フォルダを削除して同じ ID のまま作り直します。`create_level` で作ったレベルに `seed: Some(new_seed)` を渡すと、そのシード値を更新して `yoake_levels.json` にも保存します（パックや `register_level` のレベルには新しいシードを指定できません）。誰かがいるときは拒否されます。 |
| `ext::levels()` | ワールド内の全レベル ID の一覧を返します（先頭 3 つがバニラのレベルです）。読み取り可能なサーバーフックからも呼べます。 |

### サンプル `level-generator` を読む

リポジトリ内のサンプル [`crates/yoake-wasm-sdk/examples/level-generator/src/lib.rs`](../../crates/yoake-wasm-sdk/examples/level-generator/src/lib.rs) は、`init` で型・ジェネレーター・レベルを登録し、`handle_call` で段々畑のような地形（`levelgen:terraces`）を生成します。

```rust
use yoake_wasm_sdk::full::generator::{self, Fill, Request};
use yoake_wasm_sdk::full::server::{ServerMod, ext};

const TYPE: &str = r##"{
  "min_y": 0, "height": 64, "logical_height": 64, "has_skylight": true, "has_ceiling": false,
  "ambient_light": 0.2, "coordinate_scale": 1.0, "skybox": "overworld", "cardinal_light": "default",
  "attributes": {"minecraft:visual/sky_color": "#4466aa", "minecraft:visual/fog_color": "#8899cc"}
}"##;

const STEM: &str = r#"{
  "type": "levelgen:terrace_type",
  "generator": {"type": "yoake:mod", "id": "levelgen:terraces", "settings": {"step": 8}}
}"#;
```

まず `init` で 3 つを登録します。

```rust
fn init(&mut self) {
    for (what, result) in [
        ("type", ext::register_type("levelgen:terrace_type", TYPE)),
        ("generator", ext::register_generator("levelgen:terraces")),
        ("level", ext::register_level("levelgen:terraces", STEM)),
    ] {
        if let Err(e) = result {
            println!("level-generator: registering the {what}: {e}");
        }
    }
}
```

次に、チャンクが必要になるたびに、エンジンがチャンク生成ワーカースレッドから **`handle_call`** を呼び出します。`caller` は `"yoake:engine"`、`method` は `generator::METHOD`（`"yoake:generate-chunk"`）です。

```rust
fn handle_call(&mut self, _caller: String, method: String, payload: Vec<u8>) -> Result<Vec<u8>, String> {
    if method != generator::METHOD {
        return Err("unknown method".into());
    }
    let request = Request::parse(&payload).ok_or("a request this mod cannot read")?;
    let step = request.setting_int("step").unwrap_or(8) as i32;
    let sections = (request.height / 16) as u16;
    let mut fill = Fill::new();
    fill.biome("minecraft:plains");
    for index in 0..sections {
        let bottom = request.min_y + i32::from(index) * 16;
        let mut indices = vec![0u16; 4096];
        let mut any = false;
        for z in 0..16 {
            for x in 0..16 {
                let top = surface(request.x * 16 + x, request.z * 16 + z, step, request.seed);
                for y in 0..16 {
                    let wy = bottom + y;
                    let block = if wy == request.min_y {
                        3 // bedrock
                    } else if wy < top - 1 {
                        1 // stone
                    } else if wy == top - 1 {
                        2 // grass
                    } else {
                        0 // air
                    };
                    if block != 0 {
                        any = true;
                    }
                    indices[((y * 16 + z) * 16 + x) as usize] = block;
                }
            }
        }
        if any {
            fill.palette(
                index,
                &["minecraft:air", "minecraft:stone", "minecraft:grass_block", "minecraft:bedrock"],
                indices,
            );
        }
    }
    Ok(fill.to_bytes())
}
```

### チャンクジェネレーターの仕様（`Request` / `Fill` / 制限）

- **`Request` の中身**: JSON で `level`、`generator`、`x`、`z`、`min_y`、`height`、`seed`、`settings` が渡されます。数値設定は `request.setting_int("キー")` で読めます。
- **`Fill` の作り方**:
  - `fill.biome("minecraft:plains")`: チャンク全体のバイオームを指定します（省略時は `minecraft:plains`）。
  - `fill.uniform(index, "minecraft:stone")`: セクション `index`（レベル下端 `min_y` が `0`、上限が `height / 16 - 1`）を 1 種類のブロック（デフォルト状態）で埋めます。
  - `fill.palette(index, palette, indices)`: 1..=4096 個のブロック名パレットと、`4096` 個の `u16` インデックスを渡します。配列の添字は **`(y * 16 + z) * 16 + x`**（各 0..16）の順です。
  - 指定しなかったセクションは空気のままになります。`fill.to_bytes()` でエンジンが読むリトルエンディアンのバイナリ列（バージョン 1、上限 1 MiB）になります。
- **純粋関数として書く**: 同じ `(x, z)`、`seed`、`settings` に対して毎回同じ結果を返すようにしてください。`handle_call` の呼び出し中はサーバーのホストサービス（ブロック読み書きなど）にはアクセスできません。
- **時間制限と失敗時の挙動**:
  - 1 回の呼び出し上限は通常のサーバー呼び出しと同じ **25 ms**（および燃料 1000 万単位）です。ただし、ワールド開始時に多数のチャンクを一斉生成しても MOD が無効化されないよう、50 ms ごとの**累積時間制限（aggregate budget）からは免除**されています（100 ms を超えると警告ログが出ます）。別フックがモジュールを掴んでいる場合は最大 **2 秒**待ちます。
  - 返却データはブロックを 1 つも書き込む前に全体が検証されます（バージョン、セクション番号の範囲と重複、未知のブロック名、パレットサイズ `1..=4096` と範囲外インデックス、余分なバイト列など）。
  - 検証エラー、`Err` の返却、時間切れ・トラップ・無効化が起きた場合、そのチャンクは**すべて空気（空チャンク）**になり、レベルごとに 1 回だけエラーログが出ます（再試行はされません。トラップや時間切れのときはその MOD 自体も無効化されます）。

---

## 13-5. 現在の制限と未実装の点

現時点では、次の制限や未対応の箇所があります。

1. **`yoake:mod` ジェネレーターに装飾・構造物・遠景（LOD）はない**: `yoake:mod` の出力はそのまま最終的なブロックとバイオームになり、その上にバニラの洞窟カーバー・木や鉱石の配置（decoration）・構造物は乗りません。また遠景 LOD サンプラーがないため、読み込み範囲より外の遠景は空になります。気候やノイズを問い合わせるコールバックもまだありません。
2. **カスタムバイオームの追加は未対応**: データパックでゲームが知らない新しいバイオーム ID を追加しても、警告を出して除外されます。
3. **コンテンツ MOD（`.hmod`）からのディメンション宣言は未対応**: `dimension/` や `dimension_type/` はワールドの `datapacks/` か WASM MOD の API を使ってください。
4. **WIT の `level-dimension` 列挙型は 3 種のまま**: 1.0 ABI の後方互換性を保つため、フック引数に含まれる `level-dimension` 列挙型はバニラ以外のレベルでも `overworld` を返します。実際のレベル ID を知りたいときは、`levels()` や、サウンド・パーティクル・ドロップなどの文字列ペイロードを使ってください（`current-level` サービスは未追加です）。
5. **フルアクセス（DLL）向けのエンジンバインディング・フックは未公開**: `yoake-bindgen` 向けの `worldgen::generate_chunk` などのフックやバインディングはまだ用意されていません。
6. **ワールドボーダー・時刻・天候の共有**: 作成したレベルは専用のワールドボーダーを持たず、時刻や天候などの一部状態はワールドと共有されます。また一部のモブの昼判定（ホタル・クリーキング・ハチなど）は固定ティック範囲を見ており、カスタムの `timelines` トラックは読みません。
7. **削除した作成済みレベルの型は再起動まで残る**: 実行中に作成して削除したレベルの `dimension_type` は、既存の型インデックスがずれないようにサーバーを再起動するまで新規参加者向けレジストリに残ります。
8. **サーバーモジュールのホットリロード非対応**: `server-full` モジュールの `init` は起動時／ワールド開始時に動くため、すでに開いているワールドへ後からロードした MOD の `init` 登録は次のワールドまで反映されません。
9. **独自シード値レベルのクライアント側バイオームぼかし**: レベル固有の `"seed"` を設定したとき、クライアントのバイオーム境界のぼかし計算はワールド共通のハッシュシードを使うため、バイオームの境界付近の見た目がサーバーの実際のバイオーム判定とわずかにずれることがあります（置かれるブロックやサーバー側の判定自体はレベルのシード値どおりです）。
