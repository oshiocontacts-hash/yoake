# 8. Yoake Mixin 入門

Yoake Mixin は、署名済み（開発者モードでは未署名も可）の `.yak` パッケージの WebAssembly コンポーネントから、エンジン関数の決められた場所へ割り込む仕組みです。引数や戻り値を変えたり、元の処理の代わりに結果を返したりできます。[^overview]

この章では、サンプルの `mixin-demo` を使い、割り込みの書き方と守るべき決まりを見ていきます。

---

## 8-1. Mixin でできること・できないこと

Mixin の割り込み先は、エンジンがカタログに公開した関数だけです。現在のカタログには **51 個**の割り込み口（サイト）があり、どのサイトも HEAD・RETURN・overwrite を許可しています。[^catalog]

- **できること**: 公開された引数を読む、HEAD で引数を置き換えるか処理をキャンセルする、RETURN で戻り値を置き換える、overwrite で本体の代わりに結果を返す。
- **できないこと**: 任意の Rust 関数を呼ぶ、メモリのアドレスを参照する、カタログにない関数へ割り込む。
- **対象の範囲**: エンティティのティックや死亡、動物のターゲット判定、投射物やアイテムの生成などのサイトはありますが、「エンティティ全体」「AI 全体」などをまとめて捕まえる入口はありません。各サイトの関数だけが対象です。

新しい割り込み口をエンジンへ追加するにはエンジンの更新が必要です。MOD 作者は、すでにカタログにある割り込み口を選んで使います。パッケージの署名・identity・fingerprint（エンジンの識別情報）・ABI（エンジンと MOD の間のバイナリ互換性の約束）・エンジンバージョンの検査は、Mixin でも省略されません。[^overview]

## 8-2. 割り込みの種類

SDK では、1 つの `Handler` に複数の種類を設定できます。登録データでは HEAD がビット 1、RETURN がビット 2、overwrite がビット 4 です。[^sdk]

| 種類 | SDK のコールバック | 役割 |
|---|---|---|
| HEAD | `fn(&[Value]) -> HeadOutcome` | 本体の前に引数を見る・換える・キャンセルする |
| RETURN | `fn(Value) -> Option<Value>` | 現在の戻り値を受け取り、そのままにするか置き換える |
| overwrite | `fn(&[Value]) -> Value` | 本体を実行せず、戻り値を自分で作る |

### HEAD: 本体の前に割り込む

`mixin-demo` のチャット記録です。引数の 1 番目が文字列ならログへ出し、`Proceed` で元の処理を続けます。[^demo]

```rust
fn log_chat(arguments: &[Value]) -> HeadOutcome {
    if let Some(Value::Text(message)) = arguments.get(1) {
        println!("[mixin-demo] chat: {message}");
    }
    HeadOutcome::Proceed
}
```

HEAD の戻り値は `HeadOutcome::Proceed`、`HeadOutcome::ReplaceArguments(Vec<Value>)`、`HeadOutcome::Cancel(Value)` の 3 つです。引数の配列にはレシーバーも含まれ、レシーバーは 0 番目です。`Cancel` は残りの HEAD・overwrite・本体を飛ばしますが、RETURN はその結果に対して実行されます。

### RETURN: 戻り値だけを変える

次は `docs/yoake-mixin.md` の例で、落下ダメージを半分にします。`None` は現在の値を保ち、`Some(value)` が値を置き換えます。[^overview]

```rust
use yoake_wasm_sdk::mixin::{Handler, Value};

fn halve_fall_damage(result: Value) -> Option<Value> {
    match result {
        Value::Signed(damage) => Some(Value::Signed(damage / 2)),
        _ => None,
    }
}

static HANDLERS: &[Handler] = &[Handler {
    site: "world::living_fall::calculate_fall_damage#9a9a629c2a289ab6ffb883cb0cd5891af74587992a6e2da94d8090723ede5b94",
    head: None,
    returned: Some(halve_fall_damage),
    overwrite: None,
}];

yoake_wasm_sdk::export_mixin!(HANDLERS);
```

SDK のテーブルでは、RETURN のコールバックを指定するフィールド名が **`returned`** です。`Some(halve_fall_damage)` を設定し、最後に `export_mixin!` でテーブルを公開します。

### overwrite: 本体を置き換える

SDK のフィールドと関数型は次の通りです。[^sdk]

```rust
    pub overwrite: Option<fn(&[Value]) -> Value>,
```

たとえば実行時テストでは、overwrite が `Value::Signed(55)` を返し、本体の呼び出し回数が 0 になることを確認しています。同じサイト・同じサイドで overwrite を登録できるのは 1 つの MOD だけです。複数 MOD の競合については 8-7 を見てください。[^runtime]

## 8-3. SDK と `yak.toml` の設定

サンプルの `Cargo.toml` では、SDK の `default-features` を無効にし、`std` と `mixin` を有効にしています。[^demo]

```toml
yoake-wasm-sdk = { path = "../..", default-features = false, features = ["std", "mixin"] }
```

`yak.toml` では `interface = "1.1"` と `world = "inject-mod"` を指定し、必要なサイドのコンポーネントを用意します。`[mixin] targets` には、**ハッシュを含む完全なサイト ID** を書きます。

```toml
[wasm]
interface = "1.1"
world = "inject-mod"
client = "bin/wasm/mixin-demo.wasm"
server = "bin/wasm/mixin-demo.wasm"
permissions = ["service.commands", "service.chat"]

[mixin]
targets = [
    "world::living_fall::calculate_fall_damage#9a9a629c2a289ab6ffb883cb0cd5891af74587992a6e2da94d8090723ede5b94",
    "world::level::bonemeal::crop_bonemeal_increase#b61d671fb9375f084f2bf026f466add0e8423efd996180dee3fb52ca18348fd1",
    "game::chat::handle_chat_input#2d6b616020cec7006f8b8c6aeb2d8f9faf0caa7a8dbeb7f38b5a4d839b0d2e62",
]
```

`targets` は重複のない 1〜1024 件です。サーバー専用 Mixin も作れます。`side = "both"` のサイトは、パッケージにある client・server の一方または両方で実行できます。各コンポーネントは宣言したターゲットを登録し、ホストはそのサイドに合う登録だけを有効にします。サンプルでは同じコンポーネントを両サイドに使い、サーバー専用ターゲットは client 側で除外されます。[^manifest]

現在の `yoake build` / `yoake check` / `yoake pack` は `inject-mod` の WIT ワールドを扱えないため（`yoake check` が `inject-mod` のモジュールを検査できません）、`mixin-demo` は `cargo` で直接ビルドします。出力した `yoake_mixin_demo.wasm` を、パッケージ用の別フォルダで `bin/wasm/mixin-demo.wasm` として配置します。公式パッケージの署名と配布は通常のオーナー手順です。[^demo][^cli]

```sh
cargo build --offline -j 4 --release --target wasm32-wasip2
```

## 8-4. 割り込み口の一覧の見方

人が読む一覧は [`docs/yoake-mixin.md`](../yoake-mixin.md) の「Committed sites and signatures」、MOD の実装が参照する機械可読な一覧は [`crates/yoake-wasm/wit/inject-sites.json`](../../crates/yoake-wasm/wit/inject-sites.json) です。一覧の `Full target ID` を、そのまま `targets` と `Handler.site` に使います。

JSON の一例として、骨粉による成長量のサイトを見ます。[^catalog]

```json
  {
    "id": "world::level::bonemeal::crop_bonemeal_increase#b61d671fb9375f084f2bf026f466add0e8423efd996180dee3fb52ca18348fd1",
    "path": "world::level::bonemeal::crop_bonemeal_increase",
    "signature_hash": "b61d671fb9375f084f2bf026f466add0e8423efd996180dee3fb52ca18348fd1",
    "arguments": [
      "scoped<&mutSelf>",
      "block-state-id"
    ],
    "result": "s32",
    "side": "server",
    "allow": [
      "head",
      "return",
      "overwrite"
    ],
    "source": "crates/horizon-world/src/level/bonemeal.rs",
    "argument_rules": [
      {
        "kind": "finite"
      },
      {
        "kind": "block-state",
        "registry": "live"
      }
    ],
    "result_rule": {
      "kind": "number",
      "maximum": 25.0,
      "minimum": 0.0
    },
    "required_capabilities": [],
    "client_required_capabilities": []
  },
```

- `id`: `path#signature_hash` の完全な ID。`signature_hash` は引数と戻り値の形から作られ、古いハッシュは読み込み時に拒否されます。
- `arguments` / `result`: コールバックへ渡る引数と戻り値の形。`scoped<...>` は後述のスコープ付きハンドルです。
- `side`: `client`、`server`、または `both`。
- `allow`: 使える割り込みの種類。ここでは 3 種類すべてです。
- `argument_rules` / `result_rule`: 置き換える値の検査ルール。この例では成長量が 0〜25 です。
- `required_capabilities` / `client_required_capabilities`: 登録に必要なサービス権限。この例ではありません。

骨粉の成長量の 2 倍化は、このサイトだけにかかります。通常のランダムティックによる作物の成長まで 2 倍になるわけではありません。なお、`mixin-demo` はほかに落下ダメージの半減とチャットの記録も行います。[^demo]

## 8-5. 値の範囲と拒否

置き換えた数値は、カタログの型の幅と範囲に収まっている必要があります。`usize` / `isize` はホストの幅に従います。NaN と無限大は、ベクトルの中も含めて既定で拒否されます。範囲外の値は**丸められず、拒否されます**。[^overview]

主な範囲は次の通りです。ここにない作成・経験値・エンチャント・落下やジャンプの入力・攻撃速度などの範囲は、各サイトの `argument_rules` と `result_rule` を確認してください。

| 置き換える値 | 両端を含む範囲 |
|---|---|
| 爆発半径 | 0〜128 |
| 睡眠タイマー | 0〜110 |
| 作物の成長速度 | 0.01〜1024 |
| 骨粉の成長量 | 0〜25 |
| 火の着火確率 | 0〜10,000 |
| ダメージ量 | 0〜1,000,000 |
| 空腹度 / 隠れ満腹度の変化 | 0〜40 |
| 食料の栄養 | 0〜20 |
| 作成・精錬の結果の個数 | 0〜99（出力アイテムの最大スタック数も超えない） |
| 座標 / アイテム速度（各成分） | -30,000,000〜30,000,000 / -1024〜1024 |

`block-pos` は符号付き 32 ビット整数 3 個のリスト、`vec3` と `vec3-f32` は浮動小数点数 3 個のリストです。`block-state-id` は符号なし 32 ビットで、現在のワールドの状態数より小さい必要があります。この生レジストリ境界を検査するのは、`on_place` の 2 つの状態引数、`crop_random_tick`、`crop_bonemeal_increase`、`drop_falling_block` の状態引数です。境界を取得できない場合も置き換えは拒否されます。

`scoped<...>` のハンドルは、生のアドレスではなく、その呼び出しだけで有効な読み取り専用の値です。HEAD で引数を置き換えるときは、scope・type・snapshot（ハンドルが持つ識別情報）を含む元のハンドルをそのまま戻します。保存して後で使う、別の呼び出しのものを使う、内容を改ざんする、といった使い方は拒否されます。

トラップや不正な応答を返すと、そのサイト・サイド・割り込みポイントのハンドラーが無効化され、MOD・ターゲット・失敗がログに記録されます。無効化しておく期間は（未確定）です。その呼び出しは、8-8 のネイティブフォールバックで処理されます。一方、エンジンの引数やネイティブの結果が変換上限を超えた場合は、コールバックをスキップしてネイティブ処理へ戻るだけで、ハンドラーは無効化されません。[^runtime]

## 8-6. コマンド・チャットへの割り込み

対象は `game::commands::execute::execute_line`、`game::commands::execute_command`、`game::chat::handle_chat_input` です。いずれの割り込みポイントでも、サービス権限（`[wasm] permissions = ["service.commands"]`）が必要です。client 側のチャット割り込みには、さらに `"service.chat"` が必要です。テキストを読むだけのサンプルでも同じ宣言が必要です。[^catalog] なお、`service.chat` の宣言には `wasm.client` が必要で、`service.commands` は `inject-mod` 以外の WIT ワールドでは `wasm.server` が必要です（Mixin の `inject-mod` は例外です）。[^manifest]

Mixin が書き換えたコマンドは、コマンドを実行する一番末端（関数や入れ子の `run` を含む）で、禁止コマンド一覧（`stop`・`op`・`ban`・`transfer`・`publish` など。大文字小文字を区別せず、`minecraft:` 付きや入れ子の中も含みます）と照合されます。書き換えた引数は、64 KiB 以下・制御文字なしの 1 行で、`execute ... run` や `return run` の入れ子は 16 段までです。クライアントのチャットも、Mixin が書き換えた行に制御文字が含まれていれば拒否されます（ゲーム側のチャット処理 `permitted_chat_message` で確認）。リテラルの `run` 引数（たとえば `/say please run stop`）は入れ子のコマンドとは見なされません。拒否されるとコマンド失敗のメッセージが表示されます。プレイヤーが書き換えずに送った元のテキストはこの検査の対象外で、サービス権限でこの検査を回避することもできません。[^overview]

## 8-7. 順番と競合

ハンドラーは **`(bare mod ID, side)`** の順に並びます。表示用に整形した ID/サイド文字列ではなく、ID だけなので `foo` は `foo-extras` より先です。[^runtime]

- **HEAD**: 前の MOD が変更した引数を、後の MOD が受け取ります。`Cancel` は残りの HEAD・overwrite・本体を飛ばします。
- **overwrite**: 同じサイト・同じサイドで 2 つの MOD が登録すると、読み込み時の競合になります。client と server の overwrite は別々に扱われます。
- **RETURN**: MOD ID 順に実行され、前の MOD が置き換えた結果を後の MOD が受け取ります。HEAD がキャンセルした結果や overwrite の結果にも使えます。

つまり、同じサイトでも HEAD と RETURN は複数 MOD の変更を順に重ねられますが、本体を丸ごと置き換える overwrite は同じサイドで 1 つだけです。

## 8-8. 上限とトラップ

主な時間・メモリの上限は次の通りです。集計実行時間の strike（制限時間を使い切った回数）が連続 20 回になると無効化されます。[^overview]

| 上限 | Client | Server |
|---|---:|---:|
| 呼び出しごとの期限 | 20 ms | 25 ms |
| 50 ms ごとの集計実行時間 | 10 ms | 25 ms |
| 呼び出しごとの fuel | 10,000,000 | 10,000,000 |
| リニアメモリ | 64 MiB | 64 MiB |

さらに、コールバックの値の変換には、合計 4096 ノード・深さ 32・ペイロード 256 KiB の上限があります。

**トラップや不正な応答が起きても、ゲーム全体が止まるわけではありません。** そのサイト・サイド・割り込みポイントだけを無効化し、ログを出してネイティブ処理へ戻ります。無効化しておく期間は（未確定）です。フォールバックは失敗した場所で異なります。[^runtime]

1. **HEAD / overwrite の失敗**: 元の引数で本体を 1 回実行します。
2. **HEAD の引数変更後に本体を実行済みで、RETURN が失敗**: キャッシュしたそのネイティブ結果を使います。本体は再実行せず、前の RETURN が置き換えた値も使いません。
3. **HEAD がキャンセル、または overwrite 成功後に RETURN が失敗**: まだ呼んでいない本体を、元の引数で 1 回実行します。

すでに起きたゲスト・ネイティブ側の副作用は巻き戻りません。トラップのあとは、次に残っているコールバックのときに MOD が作り直され、MOD の中に持っていた状態は初期化されます。集計値は保持し、再作成時間も計上します。再作成に失敗するとそのインスタンスを無効にします。

## 8-9. MOD が無いときの速さ

サイトにハンドラーが無いときは、relaxed なアトミックロード 1 回と分岐だけで元の本体へ進みます。スレッドローカルのサイド確認、ロック、スナップショット作成、引数構築、メモリ確保の前に分岐します。[^macro]

開発記録の Windows リリースビルドのマイクロベンチマークでは、11 回の中央値で、極小の関数が **0.6194 ns/回**、実際の `calculate_fall_damage` に相当する関数が **0.7934 ns/回**の追加でした。それぞれ 40.53%・9.84% で、短い関数では相対コストを測れます。ただし、これはゲーム全体の性能測定ではありません。MOD 無しのエンジン全体のオーバーヘッドが 1% 未満かどうかは（未確定）です。[^overview]

## 8-10. よくある失敗

| 症状 | 確認すること |
|---|---|
| サイトが未知、署名が違うと拒否される | `path` だけでなく、現在のカタログの完全な `path#signature_hash` を `targets` と `Handler.site` の両方にコピーする |
| 必要なサイドで動かない | `side` と `wasm.client` / `wasm.server` を確認する。両方に同じ Wasm を置けるが、サーバー専用ターゲットは client では除外される |
| 登録そのものが拒否される | 空の登録、重複登録、未宣言サイト、許可されていない割り込みポイント、1.1 以外の interface、`inject-mod` 以外の WIT ワールドがないか確認する |
| overwrite を入れたら競合する | 同じサイト・同じサイドに別 MOD の overwrite がないか確認する。HEAD や RETURN で済ませられないか考える |
| 値を返したのに無効化される | `Value` の種類、整数の幅、サイトごとの範囲、NaN・無限大、ブロック状態 ID の生レジストリ境界を確認する |
| ハンドル付きの引数を換えられない | 元のハンドルをそのまま残す。保存・使い回し・書き換えはできない |
| 作物の成長が思ったように変わらない | `crop_bonemeal_increase` は骨粉の増分だけを変える。通常の `crop_random_tick` とは別のサイト |
| コマンド・チャットだけ登録できない | `service.commands` と、client チャットなら `service.chat` を宣言し、`service.chat` の `wasm.client` と、`inject-mod` 以外での `service.commands` の `wasm.server` を確認する |

---

## 次のステップ

Mixin の書き方が分かったら、[README](./README.md) に戻って、ほかの章をもう一度見直したり、サンプルの `mixin-demo`（[4. サンプル MOD 集を読む](./04-examples.md)）を動かしたりしてみましょう。

[^overview]: 確認元: `docs/yoake-mixin.md`（対象範囲、値、失敗時の動作、上限、ベンチマーク）。
[^sdk]: 確認元: `crates/yoake-wasm-sdk/src/mixin.rs`（`Handler`、コールバックの型、登録ビット、`export_mixin!`）。
[^demo]: 確認元: `crates/yoake-wasm-sdk/examples/mixin-demo/` の `src/lib.rs`、`Cargo.toml`、`yak.toml`、`README.md`。
[^manifest]: 確認元: `crates/horizon-mods/src/yak/manifest.rs` の `Mixin`、`Wasm`、`Manifest::validate`、および `docs/yoake-mixin.md` の「Package and registration」。
[^catalog]: 確認元: `crates/yoake-wasm/wit/inject-sites.json` と `crates/horizon-mod-api-gen/src/inject_catalog.rs`。
[^runtime]: 確認元: `crates/yoake-inject-rt/src/lib.rs`、`src/tests.rs`、`src/security_tests.rs`、`tests/macro_dispatch.rs`、および `docs/yoake-mixin.md` の「Ordering, conflicts, and failure」。
[^macro]: 確認元: `crates/yoake-inject-macro/src/lib.rs`、`crates/yoake-inject-rt/src/lib.rs`、および `docs/yoake-mixin.md` の「No-injection cost and measurement」。
[^cli]: 確認元: `crates/yoake-cli/src/main.rs` の `check` にあるワールドの検査。扱えるのは `client-mod`、`client-full`、`server-full` だけです。
