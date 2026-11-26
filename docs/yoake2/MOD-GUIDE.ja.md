# Yoake 2.0 フルアクセス MOD の作り方

このガイドは **フルアクセス MOD** を作りたい人向けです。フルアクセス MOD とは、`.yak` ファイルに入れた WebAssembly モジュールのことで、Java の MOD のようにエンジンの関数を呼び出したり、フックしたりできます。動くひな形は [`templates/full-access-mod`](../../templates/full-access-mod/README.ja.md) にあります。英語版は [MOD-GUIDE.md](MOD-GUIDE.md) です。

ここに書いたことは、このツリーのコード（コミット `4b707d438` と、このガイド自身のコミットで加わったもの）に照らして確認しています。確かめられなかったことは、推測で書かずに末尾の「要確認」にまとめました。

参照元は [ENGINE_CALL_ABI.md](ENGINE_CALL_ABI.md)（ABI）と SDK（`crates/yoake-guest-sdk`）です。このガイドはステータスコードを全部は繰り返しません。MOD を書くのに必要なことと、つまずきやすい点を書いています。

## 1. フルアクセス MOD とは、そして信頼のしくみ

フルアクセス MOD は、エンジンがゲームのすぐ隣で読み込む WebAssembly の **コアモジュール**（コンポーネントではありません）です。入口は `yoake_init` と `yoake_hook` の 2 つ、インポートは `env` モジュールだけです。できることは次のとおりです。

- **エンジンの関数を呼び出す**: 生成されたバインディング（`engine_call`）を通して、エンジンが渡すハンドル（フックの引数、または `Game` ルートからのパス）に対して呼びます。
- **エンジンの関数をフックする**: `yoake_init` でフック ID を登録しておくと、その関数が実行されるたびにエンジンが `yoake_hook` を呼びます。フックの中では元の処理を実行する（`original`）、結果を置き換える、元の処理を飛ばす、のどれもできます。
- **エンジンが用意する合成 tick で動く**: サーバー tick（シミュレーションされているワールドの tick）。クライアント tick と HUD 描画のフックは別ブランチにあります（第 6 章）。

使い心地は Java の MOD に近く、ファイルを入れて遊ぶだけです。そのぶん代償も Java の MOD と同じです。**フルアクセス MOD はエンジン自身と同じ力を持ちます。** ランチャーの確認ダイアログにもはっきり書かれています。MOD はゲームそのものと同じ権限を得て、ワールドや設定の変更、PC 上のファイルの読み書き、ネットワークの利用までできてしまい、「Yoake には MOD が何をするか確かめられない」という内容です。

誰が何を決めるのか、平たく言うと次のとおりです。

- **署名されるのはエンジンの実行ファイルだけ**です。フルアクセス MOD には署名がありません。
- **権限を持つのはランチャー**です。プレイヤーは `.yak` を入れ（Add a .yak file…、またはドラッグ＆ドロップ）、Mods ページで確認を経てから **Allowed**（許可）に切り替えます。
- **動くのは、許可されたそのバイト列だけ**です。許可には MOD の ID、バージョン、`.yak` ファイルの SHA-256 が記録されます。作り直してバイト列が変われば、プレイヤーはもう一度許可しなければなりません。ランチャーはそのファイルを「The file changed since you allowed it · will not run until allowed again」と表示します。
- **サーバーや友達から届いたパッケージは、ハッシュが一致していてもフルアクセスになりません。** `mods` フォルダに手で置いたファイルも同じです（インストール記録がないため）。動かせるのは、プレイヤー自身が自分の PC から入れたもの（`local`）か、Yoake カタログのもの（`catalog`。許可済み、または信頼された鍵で署名されたもの）だけです。
- ゲーム本体はランチャーの管理ファイルを読みません。ランチャーが起動のたびにゲームへ一覧を渡します。MOD が管理ファイルを書き換えても、動作中のゲームには何も起きません（変更は次の起動で差分として表示され、プレイヤーが承認する必要があります）。

つまり作者にとっては、プレイヤーが信頼するのは **あなた** と **このファイル** です。できればビルドを再現できるようにして、MOD が何をするのかを書いておいてください。

## 2. セットアップ: ツールチェーンとターゲット

Rust ツールチェーン（`rust-toolchain.toml` が **rustc 1.100.0** とターゲット `wasm32-unknown-unknown`・`wasm32-wasip2` を固定しています）と、次のどちらかを使います。

- **`yoake` CLI（`crates/yoake-cli`）**: どこでも `yoake new full-access my-mod` を実行すると、バンドル済みの `yoake-guest-sdk` 付き Cargo プロジェクトが作られます。あとは `yoake build`・`yoake check`・`yoake pack` でビルド・検査・パックまで完了します。
- **ソース一式内の `rustc` スクリプト（`templates/full-access-mod`）**: `build.sh` / `build.ps1` で `rustc +1.100.0` を直接呼び、`cargo-horizon-mod` でパックします。

GNU ツールチェーンの Windows では、`yoake` や `cargo-horizon-mod` のビルドに `dlltool` とアセンブラが `PATH` に必要です。MOD 自体のビルドには `rustc` と同梱の `rust-lld` だけで足ります。`wasm-tools`・`cargo-component`・`wasm-opt` は使いません。

## 3. プロジェクトの構成

`yoake new full-access my-mod` を使うと、`yoake build`・`yoake check`・`yoake pack` にそのまま対応した Cargo プロジェクトが作られます。

```text
my-mod/
  yak.toml                 パッケージのマニフェスト（world = "full-access", interface = "1.7"）
  Cargo.toml               .yoake/yoake-guest-sdk にパス依存
  src/lib.rs               #![no_std] cdylib の MOD 本体
  .yoake/yoake-guest-sdk/  同梱された SDK のコピー
  bin/wasm/my-mod.wasm     ビルド出力（yoake build が書き出す）
  dist/my-mod-0.1.0.yak    できあがったパッケージ（yoake pack が書き出す）
```

手動の `rustc` スクリプト構成（`templates/full-access-mod`）では、`Cargo.toml` と `.yoake/` の代わりに `build.sh` / `build.ps1` を使います。どちらでも、MOD は `yoake-guest-sdk` に対してコンパイルする `#![no_std]` の cdylib クレートです。

```rust
#![no_std]
extern crate alloc;

use yoake_guest_sdk::{Handle, Registrar, export_guest, hooks, original};

fn init(r: &mut Registrar) { r.hook(hooks::SERVER_TICK, 0).expect("register SERVER_TICK"); }
fn hook(id: u32, root: Handle) -> i64 { let _ = original(root); 0 }

export_guest!(init, hook);
```

`export_guest!(init, hook)` が 2 つのエクスポート（`yoake_init`、`yoake_hook`）と ABI のカスタムセクション（第 13 章）を出力します。アロケータ（`alloc` が使えます）と、トラップするパニックハンドラは SDK が用意します。`std` はありません（`wasm32-wasip1` でビルドする場合を除く。第 15 章を参照）。

## 4. `yoake_init` とフックを書く

### 2 つの関数

`export_guest!(init, hook)` には 2 つの関数を渡します。

| 関数 | 呼ばれるとき | 役割 |
|---|---|---|
| `init(r: &mut Registrar)` | MOD の読み込み時に 1 回（`yoake_init`） | `r.hook(id, priority)` で欲しいフック ID を登録する |
| `hook(id: u32, handle: Handle) -> i64` | 登録したフックが発火するたび（`yoake_hook`） | 実際の処理。`match id` で処理を選ぶ。戻り値がフック対象の関数の結果になる |

フック ID のうち小さい数字は、エンジンの関数や呼び出し箇所の **台帳 ID** です。`0x8000_0000` 以上はエンジンが用意する合成フックで、このツリーでは `hooks::SERVER_TICK`（`0x8000_0001`）がそれです。台帳は `hook-ledger/`（`horizon_client.json`、`horizon_client_lib.json`）にあり、各エントリが `id`、`key`（関数の Rust パス）、`status`（`live` か `tombstone`）、`kind`（`fn` か `callsite`）を持ちます。登録してよいのは `live` の ID だけです。エンジンが実際に入れる ID は生成されたブリッジ（`crates/horizon-client/src/yoake_glue/hook_bridge_*.rs` の `BRIDGED`、`OBSERVE_ONLY`、`UNBRIDGED`）で決まります。このコミットでブリッジされているのは 201 個（関数フック 156、呼び出し箇所フック 45。うち 67 個は `ObserveOnly`）で、4 つのモジュールに分かれます（`entities` 81、`game::item_use` 29、`mobs` 68、`motion` 23）。読み込み時に `hook_register` が結果を返します（次のモードの表を参照）。

### `Registrar::hook` とモード

`r.hook(id, priority)` は `Result<HookMode, RegisterError>` を返します。

| `HookMode` | 意味 |
|---|---|
| `Full` | 戻り値がそのままフック対象の関数の結果になる。`original` を呼ばなければ呼び出しを取り消せる |
| `ObserveOnly` | 関数の結果の型が `i64` に収まらない。戻り値は無視され、呼び出しを取り消すこともできない |
| `NotCallable` | 登録はされたが、このエンジンビルドにはその ID を呼ぶ手段がない。フックは一度も動かない（ローダーが知らせる） |

| `RegisterError` | 意味 |
|---|---|
| `NotInInit` | `hook_register` が使えるのは `yoake_init` の実行中だけ |
| `TooMany` | 1 つの MOD で登録できるのは 4096 個まで |
| `ReservedId` | ID 0 は予約済み |
| `Unknown(n)` | それ以外のステータス |

`priority` は小さいほど先に動きます。同じフック ID を登録した MOD は `(priority, 読み込み順)` の順に並び、先頭が一番外側です。その `original` が次の MOD を呼び、最後の MOD の `original` がエンジンを呼びます。

3 つのフックを登録してモードを報告する例です（`crates/yoake-guest-sdk/examples/tick_and_item_use.rs` より）。

```rust
fn init(r: &mut Registrar) {
    let tick_mode = r.hook(hooks::SERVER_TICK, 0).expect("register SERVER_TICK");
    let full_mode = r.hook(HOOK_START_USING_ITEM, 0).expect("register start_using_item");
    let observe_mode = r.hook(HOOK_EXPERIENCE_GIVE, 0).expect("register Experience::give");
    // ...
}
```

### `original` と結果

`original(handle) -> Result<i64, Error>` は、チェーンの次の MOD を実行します。自分が最後ならエンジン本来の関数を実行します。**`original` の実行中は、この activation のハンドルがすべて `Locked` になります。** 必要な値は前に読んでおくか、後で新しいパスハンドルを取り直してください。

**Full** のフックでは次のようにします。

- `original` を呼んで、その値をそのまま返す（素通し）か、自分の `i64` を返す（置き換え）。
- `original` を呼ばずに返すと、呼び出しを **取り消し**、チェーンの残りも飛ばします。返した値が結果になります。
- 値は結果の型に収まる必要があります。`()` は `0`、`bool` は `0` か `1`、64 ビットより狭い整数はその数値、`u64` は 2 の補数のビットパターン、`f32` は `0..=u32::MAX` の値としてのビット列、`f64` は 64 ビットのパターンです。範囲外の値（たとえば `bool` に `2`）は拒否され、エンジンは代わりに元の結果を使って `yoake: a mod returned N for hook ID, out of range for its result type; the original result is used` と出力します。
- `original` を 2 回呼ぶと、ふつうはエンジンの関数も 2 回動きます。**例外** は、フック対象の関数が値渡しの引数を取り、ブリッジがその引数をムーブするしかないフックです（このビルドでは ID 16、61、62、130、168、172、175、433、434、473、484、501、506、515）。これらでは関数は最大 1 回しか動かず、2 回目の `original` は `0` を返します。モードでは見分けられません（どれも `Full` です）。関数名つきの一覧は [ENGINE_CALL_ABI.md の「Hooks whose original runs at most once」](ENGINE_CALL_ABI.md#hooks-whose-original-runs-at-most-once) にあります。結果を 2 回使いたいときは、`original` をもう一度呼ばずに最初の結果を取っておいてください。

```rust
HOOK_START_USING_ITEM => {
    // Full mode: pass the original's value through, or replace it with your own i64.
    let result = original(handle).unwrap_or(0);
    result
}
```

**ObserveOnly** のフックでは、前後の値を読むことはできますが、戻り値は無視され、呼び出しの取り消しもできません。`original` を一度も呼ばなくても、あなたが戻ったあとにエンジンが本来の関数を 1 回実行します。`original` は何回呼んでもかまいませんが、本物の元の処理が走るのは 1 回の呼び出しにつき **最大 1 回** です。2 回目以降は `0` を返し、最初の結果が保たれます。

```rust
HOOK_EXPERIENCE_GIVE => {
    let before = entities_experience_total_get(handle).unwrap_or(-1);
    let _ = original(handle);
    // `handle` is Locked only while `original` is running; afterwards it is usable again.
    let after = entities_experience_total_get(handle).unwrap_or(-1);
    0
}
```

### トラップと、よくある落とし穴

- **トラップするとその MOD は無効になります。** パニック、範囲外アクセス、`unreachable`、あなたの渡した引数が原因のエンジン側パニックなどでゲスト側がトラップすると、その MOD はセッションが終わるまですべてのチェーンから外れます。周りの MOD とエンジンは、1 回ずつきちんと動き続けます。
- **`original` をまたいで借用を持たないでください。** エンジンは、同じゲストに再び入るフック（同じインスタンスの入れ子の activation）を呼ぶことがあります。`RefCell` のガードや `static mut` への参照を `original` の間じゅう持っていると、入れ子のフックが動くときに借用済みになっていてパニックし、パニックはトラップです。必要な値をコピーして取り出してから呼んでください。
- **状態はアトミックで持ちましょう**（例では `AtomicU32`）。ゲストはシングルスレッドですが、フックは入れ子になるので、借用ガードが要るものは避けてください。
- **`hook_register` は `yoake_init` の中だけ**です。
- **ハンドルは MOD ごと、activation ごとのものです。** `Handle` を static に保存しないでください。次の activation では `Stale` です。
- **`report` を毎 tick 呼ばないでください。** `report(slot, value)` はテスト用の仕組みで、ホストはその組をメモリ上のリストに追加するだけです。読むのはテスト（`runtime::take_reports`）だけで、ゲームの中では誰もこのリストを空にしません。ひな形では最初の 3 tick だけ報告しています。
- **`yoake_init` には上限があります**: 燃料 5000 万、epoch 2000 tick。ここで回り続けると読み込みに失敗します。実際の処理はフックの中で行ってください。
- **ホットパスはコストがかかります。** WebAssembly のフックは 1 回につき 110〜150 ns 程度かかります（引数があればさらに増えます）。1 tick に何十万回も呼ばれる関数のフックは避けてください（エンジン側の目安は `reports/MX3S2.md`）。

## 5. エンジン関数を呼び出す

### 型付きスタブ

`engine_call` を自分で呼ぶことはほとんどありません。`yoake_guest_sdk::stubs` に、バインディングごとに生成された型付きの関数があります。

| モジュール | 中身 |
|---|---|
| `stubs::entities` | エンティティのフィールドのゲッター／セッターとメソッド。例: `entities_experience_total_get`、`entities_itementity_fire_ticks_set`、`entities_itementity_position_get` |
| `stubs::items` | アイテムのデータ型とアイテムの関数 |
| `stubs::game_items` | `Game` ルートからパスハンドルを開く `Game.<name>#handle` の関数 |
| `stubs::mobs`、`stubs::motion` | Mob・プレイヤー移動のゲッター／セッターとメソッド |
| `stubs::custom_mobs` | F4M カスタム Mob のフックと操作（`yoake_guest_sdk::custom_mobs` でラップ。第 17 章） |
| `stubs::f_services` | F ルートのクライアント HUD・IME・音声・GPU サービス（`yoake_guest_sdk::f_services` でラップ。第 6 章） |
| `stubs::mod_generator` | `worldgen::generate_chunk` 用の `ModChunk` バインディング（第 16 章） |
| `stubs::launch_options`、`render_settings`、`render_world` | 起動設定やクライアント描画状態のバインディング |

名前の付け方は、フィールドなら `<module>_<type>_<field>_get` / `_set`、メソッドなら `<module>_<type>_<method>` です。何がバインドされているかは `docs/yoake2/bindgen/`（`functions-*.md`、`coverage.md`）に、ID は `crates/yoake-bindgen/ids.lock.json`（追記のみで、再利用されません）にあります。スタブは `Result<T, Error>` を返します。

```rust
// from examples/engine_call_game_path.rs
let item = game_items_game_item_entity_handle(root, 4242)?;          // a path handle
let pos = entities_itementity_position_get(item)?;                   // a compound field (Vec3)
let ok = entities_itementity_fire_ticks_set(item, 55).is_ok();       // a setter
```

`Error` は次のとおりです。

| `Error` | 意味 |
|---|---|
| `Handle(code)` | ハンドルが使えない。1 `Stale`、2 `Locked`、3 `BadField`、4 `NoSuchArg`、5 `WrongType`、6 `LimitReached`、7 `ReadOnly`。何も実行されていない |
| `Status(n)` | 呼び出しが拒否された、または失敗した。生の負のステータス（-1 は引数が 1 MiB 超、-3 は引数の形式不正、-4 は読み取り専用フィールドへのセッター、-5 は未知のバインディング ID、-6 はディスパッチャなし、-8 は結果の喪失 など）。詳しくは ENGINE_CALL_ABI.md |
| `Decode(_)` | 結果のバイト列がスタブの型として読み取れなかった |

`err.is_stale()` は `Handle(1)` のときに true です。エラーは **トラップではありません**。処理して先へ進んでください。

### ハンドルと `Game` のパス

ハンドルは呼び出しの対象（レシーバ）です。種類は 2 つあります。

- **フックの引数**: フック対象の関数の最初の引数（引数 0）。`yoake_hook` に `handle` として渡されます。その型のゲッターとセッターが使えます。
- **パスハンドル**: 引数 0 が `Game` ルートのフック、つまり `hooks::SERVER_TICK` の中だけで使えます。ルートを渡して `Game.<name>#handle` のバインディングを呼ぶと、ハンドルが得られます。

  | バインディング（SDK の関数） | 開くもの |
  |---|---|
  | `game_items_game_experience_handle(root)` | 経験値オブジェクト |
  | `game_items_game_item_entity_handle(root, id)` | エンティティ ID で指定したドロップアイテム |
  | `game_items_game_armor_stand_handle(root, id)` | ID で指定したアーマースタンド |
  | `game_items_game_cushion_handle(root, id)`、`..._experience_orb_handle`、`..._item_frame_handle`、`..._painting_handle` | それぞれの型について同様 |

  パスハンドルは **パス**（`Game`、フィールド、ID）で、呼び出しのたびにルートから借り直されます。参照ではありません。有効なのはその activation の中だけです。対象が消えた（アイテムが拾われた、削除された）ときは `Stale`、`original` や同じ activation の別の呼び出しが実行中のときは `Locked`、別の型のメソッドを呼ぶと `WrongType` です。1 activation で最大 256 個まで。ほかのフックの中にはルートがないので、パスを求めても `WrongType` になります。

### 複数引数のフックと値渡し引数（ABI 1.1）

ABI 1.1 では、複数引数（最大 8 個）を取る関数のフックで `handle.arg(index)` により各引数のハンドルを取得できます（`handle.arg(0)` が `yoake_hook` に渡されたハンドルです）。SDK には次の関数があります。

- `arg_value(h) -> Result<i64, Error>`: 値渡しスカラー引数のビット列を読みます（オブジェクトハンドルでは `WrongKind`）。
- `set_arg_value(h, bits) -> Result<(), Error>`: 値渡し引数を書き換え、次の `original` と後続 MOD に渡します。
- `swap_args(a, b) -> Result<(), Error>`: 同じ型・同じ可変性（`&mut` 同士または `&` 同士）の 2 つのオブジェクト引数をエイリアシングなしに入れ替えます。
- `replace_arg(dst, src) -> Result<(), Error>`: 共有参照（`&T`）の引数を `src` に差し替えます（`&mut T` では `Overlap` になるため `swap_args` を使います）。

ABI 1.1 の新しいハンドルステータス: `8`（`Overlap`）と `9`（`WrongKind`）。詳しくは [ENGINE_CALL_ABI.md](ENGINE_CALL_ABI.md#every-argument-of-a-hook-abi-11-w8) を参照してください。

### `OUT_TOO_SMALL` とチケット

普段は見ることがありません。SDK の `call` が処理するからです。ただ、理由は知っておく価値があります。

- 生のインポートは `engine_call(handle, id, ptr, len, out_ptr, out_cap) -> i32` です。0 以上の戻り値は、書き込んだ結果のバイト数です。
- 結果が `out_cap` に **収まらなかった** とき、呼び出しは **すでに実行済み** で、結果は **チケット** に保管されています。戻り値は `-(チケット + 256)` 以下で、必要なサイズは `out_ptr` にリトルエンディアンの `u32` で書かれます。結果は `engine_take(ticket, out_ptr, out_cap)` で取り出します。
- **やり直しのために同じ呼び出しをもう一度しないでください。** セッター、スポーン、カウンターが 2 回実行されてしまいます。
- SDK が代わりにやってくれます。`call` はまず 256 バイトのバッファで試し、`OUT_TOO_SMALL` になったら必要な大きさだけ確保してチケットを取り出します。チケットが有効なのは、その activation の中だけです。1 activation が保持できる結果は最大 8 個、合計 16 MiB までで、それを超えるとステータス `-8`（`Error::Status(-8)`）になります。**呼び出しは実行されたのに結果は失われます。** 大きな結果をいくつも同時に保持する手書きのゲストでないと、ここには届きません。

## 6. クライアント側とサーバー側のモジュール

フルアクセスのパッケージは、`[wasm] server`、`[wasm] client`、またはその両方を宣言できます（MX3CL1、レポート `docs/yoake2/reports/MX3CL1.md`）。

| 宣言 | 動く場所 |
|---|---|
| `[wasm] server = "bin/wasm/x.wasm"` | ワールド開始時のシミュレーションスレッド |
| `[wasm] client = "bin/wasm/y.wasm"` | クライアントセッションの準備ができたときの、ウィンドウのスレッド |
| 両方 | 両方のスレッドで、別々の 2 つのインスタンスとして（メモリもグローバルも共有しない） |

`[side]` は宣言したモジュールと完全に一致させる必要があります（`side.server` は `wasm.server` があるときだけ、`side.client` は `wasm.client` があるときだけ）。クライアント側も同じ許可リストと同じルールに従います。合成フックは次のとおりです。
- `hooks::SERVER_TICK`（`0x8000_0001`）：サーバーの tick ごとに呼ばれます。引数 0 はサーバーの `Game` ルートです（Full）。
- `hooks::CLIENT_TICK`（`0x8000_0002`）：クライアントの tick ごとに 1 回呼ばれます。引数 0 はクライアントの `Game` です（Full）。
- `hooks::HUD_RENDER`（`0x8000_0003`）：HUD のあと、フレームごとに 1 回呼ばれます。観察専用で、引数 0 は無効なハンドルです（描画には `f_services::hud_draw` を使います）。
- `hooks::GPU_FRAME`（`0x8000_0004`）：レンダラーの GPU パスからフレームごとに 1 回呼ばれます（観察専用）。
- `hooks::IME_PREEDIT`（`0x8000_0005`）・`hooks::IME_COMMIT`（`0x8000_0006`）：IME の変換途中／確定イベント（観察専用）。内容は `f_services::ime_event()` で読みます。

### F ルートのクライアントサービス（`yoake_guest_sdk::f_services` / `stubs::f_services`）

クライアント側のフルアクセスモジュールは、`[wasm] permissions` なしでサンドボックス `client-full` と同じクライアントホストサービスを呼べます。
- **HUD**: `f_services::{hud_draw, hud_gui_size, hud_gui_scale}`（`hud_draw` は `HUD_RENDER` 内のみ、1 フレーム最大 8,192 個の `FHud` コマンド）。
- **音声**: `f_services::{play_sound, play_sound_at}` と RAII の `f_services::AudioStream`（`open`、`push`、`queued`、`dropped`、`move_to`。Drop で自動クローズ）。
- **IME**: `f_services::{ime_set_allowed, ime_set_cursor_area, ime_event}`。
- **GPU**: `stubs::f_services::f_services_gpu_*`（リソースは `yoake-f-client` に紐づき、クライアント MOD のアンロード時に解放されます）。

完全な例は [`crates/yoake-guest-sdk/examples/f_client_services.rs`](../../crates/yoake-guest-sdk/examples/f_client_services.rs) にあります。

## 7. 小さな MOD を最後まで

ひな形の `src/lib.rs`（短縮版）です。サーバー tick を登録し、経験値オブジェクトに対してエンジン呼び出しを行い、最初の 3 tick を報告し、tick を実行させます。

```rust
#![no_std]
extern crate alloc;

use core::sync::atomic::{AtomicU32, Ordering};
use yoake_guest_sdk::stubs::entities::entities_experience_total_get;
use yoake_guest_sdk::stubs::game_items::game_items_game_experience_handle;
use yoake_guest_sdk::{Error, Handle, Registrar, export_guest, hooks, original, report};

static TICKS: AtomicU32 = AtomicU32::new(0);

fn init(r: &mut Registrar) {
    r.hook(hooks::SERVER_TICK, 0).expect("register SERVER_TICK");
}

fn hook(id: u32, root: Handle) -> i64 {
    if id != hooks::SERVER_TICK {
        return original(root).unwrap_or(0);
    }
    let tick = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    // two engine calls, BEFORE `original`: handles are Locked while it runs
    let total = game_items_game_experience_handle(root).and_then(entities_experience_total_get);
    if tick <= 3 {
        report(1, i64::from(tick));
        report(2, total.map_or(-1, i64::from));
    }
    let _ = original(root);
    0
}

export_guest!(init, hook);
```

## 8. wasm にビルドする

### `yoake build` を使う方法（`yoake new full-access` の場合）

`yoake new full-access <name>` で作ったプロジェクト内では、`yoake build` がコアモジュール用の `rust-lld` フラグ（`--no-entry`、`--export-memory`、`--export=yoake_hook`、`--export=yoake_init`）を自動で渡し、成果物を `[wasm] server` / `[wasm] client`（および `[wasm] worker`）へコピーします。

```sh
yoake build
yoake check
yoake pack
```

### 手動で `rustc` を使う方法（`crates/yoake-guest-sdk/tests/support/build.rs`）

Cargo を使わず `rustc` を 2 回呼んでビルドする方法です。

```sh
COMMON="--edition 2024 --target wasm32-wasip2 -C panic=abort -C opt-level=1"

# 1. the SDK as an rlib
rustc +1.100.0 $COMMON --crate-type rlib --crate-name yoake_guest_sdk \
  --out-dir build/sdk  <checkout>/crates/yoake-guest-sdk/src/lib.rs

# 2. the mod as a core module (no component wrapper)
rustc +1.100.0 $COMMON --crate-type cdylib --crate-name my_full_access_mod \
  -C linker=rust-lld -C linker-flavor=wasm-ld -C strip=symbols \
  -C link-arg=--no-entry -C link-arg=--export-memory \
  -C link-arg=--export=yoake_hook -C link-arg=--export=yoake_init \
  -L dependency=build/sdk --extern yoake_guest_sdk=build/sdk/libyoake_guest_sdk.rlib \
  -o bin/wasm/my_full_access_mod.wasm  src/lib.rs
```

大事なフラグは次のとおりです。`wasm32-wasip2` に `-C linker=rust-lld -C linker-flavor=wasm-ld` を組み合わせるとコンポーネントの包みが付きません。`main` がないので `--no-entry`。`--export-memory` と 2 つの `--export=` は、ローダーが必要とするエクスポートです。パニックはトラップなので `-C panic=abort`。モジュールに WASI のインポートはありません。ひな形の `build.sh` / `build.ps1` は、これをそのまま実行してからパックします。

## 9. `.yak` にパックする

### `yak.toml`

```toml
format = 1
id = "my_full_access_mod"
version = "0.1.0"
name = "My Full-Access Mod"
authors = ["Your Name"]
description = "What it does."

[yoake]
version = ">=0.0.0"

[side]
client = false
server = true
required_on_join = false

[wasm]
interface = "1.7"
server = "bin/wasm/my_full_access_mod.wasm"
world = "full-access"
```

フルアクセス MOD についてマニフェストのチェックが強制するルールです。

| フィールド | ルール |
|---|---|
| `format` | `1` |
| `id` | `a-z`、`0-9`、`_`、`-` の 3〜48 文字 |
| `version` | セマンティックバージョン |
| `authors` | 1〜128 名 |
| `[yoake] version` | ランチャーがインストール済みの Yoake と照合するバージョン条件（`>=0.0.0` なら常に一致） |
| `[wasm] world` | `"full-access"` |
| `[wasm] interface` | `"1.7"`（または `"1.0"`。メジャーは 1、マイナー `1`/`2` は `inject-mod` 専用） |
| `[wasm] server` / `client` | `server` と `client` の少なくとも一方を指定。`bin/wasm/` の直下にある `.wasm` ファイル |
| `[wasm] permissions` | 指定不可。フルアクセスは全か無かだから |
| `[wasm] worker` | 省略可。`bin/wasm/` 直下の `worker-mod` コンポーネント（第 15 章） |
| `[wasm] host_filesystem` | 省略可。真偽値。`/host/<ドライブ文字>` を使えるようにする（第 15 章） |
| `[side]` | `side.server` は `wasm.server` の有無、`side.client` は `wasm.client` の有無と一致させる |

ほかに `name_ja`、`license`、`homepage`、`dependencies`、`optional`、`conflicts` が使えます。`yak.toml` と同じ場所にある `README.md`、`LICENSE`、`content/`、`assets/`、`shaders/`、`config/` も一緒にパックされます。

### パックする

`yoake new full-access` で作ったプロジェクトでは `yoake pack` を実行します（ビルドと `yoake check` による `yoake_abi`／インポート検証を行い、`dist/<id>-<version>.yak` を出力します）。

手動の `rustc` 構成（`templates/full-access-mod`）では、ソース一式のパックツール `cargo-horizon-mod` を使います。ソース一式のルートで次を実行します。

```sh
cargo run -p cargo-horizon-mod -- pack path/to/my-mod
```

`yak.toml` を読み、宣言されたファイルを集めて、**署名なし** の `<id>-<version>.yak` をプロジェクトのフォルダに書き出します。同名のファイルがあると上書きしません（ひな形のスクリプトは古いファイルを消し、新しいファイルを `dist/` に移します）。出力の要約行には「data-only package」と出ますが、これはネイティブバイナリ（`[bin]`）の有無を指す言い回しで、WebAssembly モジュール入りでもこう表示されます。できあがりの確認は次のとおりです。

```sh
cargo run -p cargo-horizon-mod -- yak verify path/to/my_full_access_mod-0.1.0.yak
```

## 10. ランチャーでインストールして許可する

1. ランチャーで Yoake インスタンスの **Mods ページ** を開きます。
2. **Add a .yak file…** で `.yak` を選ぶか、ファイルをページにドラッグ＆ドロップします（「Drop a .yak file here」）。ランチャーは `<instance>/mods/` にコピーし、この PC から来たもの（`local`）と記録します。この時点では **まだ許可されていません**。
3. **Full-access mods** という欄が現れ、「Full access」のタグが付いた行が表示されます。状態は「Not allowed · will not run」です。
4. スイッチを入れます。確認ダイアログに MOD の名前、バージョン、ファイルの SHA-256 の先頭 12 桁、入手元が表示され、ゲームと同じ権限を得ることが警告されます。**Allow** を押します。
5. 行が **Allowed** になります。ランチャーは **ゲームを起動するとき** に許可リストを作り、ゲームはそれを最初のワールド開始時に 1 度だけ読みます。許可したあとは、ランチャーからゲームを起動（または再起動）してください。

MOD の更新: 同じ ID の別バージョンは「already installed in another version; remove it first」と断られます。**Remove** で行を消してから新しいファイルを追加し、もう一度許可してください。同じファイル名でバイト列が変わった場合は「changed since you allowed it」になります。開発中は、作り直すたびにこの許可をやり直すことになります。

行に「Allowed · runs after you accept the changes above」と出ているときは、ランチャーが「Full-access changes since you last accepted」というパネルを表示しています。プレイヤーが一覧（Added／Changed／Removed）を確認し、**Accept these changes** を押します。プレイヤー自身のクリックによらない変更（管理ファイルが変わった、`mods` にファイルが置かれた、など）は、こうして保留されます。

## 11. デバッグ

ゲスト側に `print` はありません。使える手段は次のとおりです。

- **エンジン自身の出力。** ローダーとランタイムが MOD について言いたいことは、`yoake:` で始まる行になります（ゲームプロセスの stderr に出力され、ローダーのメッセージは `log` クレートの `warn` レベルにも出ます）。
- **戻り値。** スタブは `Result` を返すので `Error` を見てください。テストではエラーを数値にして `report` します（ひな形の `error_code` を参照）。
- **ゲームなしのハーネスで動かす。** `crates/horizon-client/tests/yoake2_template.rs` は、ひな形をビルドしてパックし、インストールと許可を行い、本物のローダーで読み込んで、仮の `Game` ルートでサーバー tick を回します。自分の MOD 用にコピーして使えます。`report` の値は `runtime::take_reports()` で受け取ります。

  ```sh
  cargo test --offline -j 4 -p horizon-client --features yoake2-slice,yoake-hook-bench --test yoake2_template
  ```

### 見える行

| 行 | 意味 |
|---|---|
| `yoake: full-access mod <file> not loaded: <reason>` | ローダーがパッケージを断った。理由は次の表 |
| `yoake: full-access mod '<id>' was built for ABI M.m, this engine speaks M.n: engine calls it makes that this engine does not know are refused` | ゲストのマイナーバージョンがエンジンより新しい |
| `yoake: full-access mod '<id>' registered hooks this engine build cannot call, they will never run: <id> (<why>)` | `NotCallable` のフック。その ID にこのビルドではブリッジがない（例: `arg0 is &self, needs &mut`） |
| `yoake: mod <id>: a hook activation has run N ms of guest time (watchdog: 50 ms); it is not stopped` | 1 回の activation でゲスト自身の時間が 50 ms を超えた。そのまま動き続け、無効にはならない |
| `yoake: engine state may be inconsistent: binding N panicked on guest-supplied arguments (...); the mod is disabled` | あなたの渡した引数でエンジン関数がパニックした。MOD はトラップして無効になった |
| `yoake: a mod returned N for hook ID, out of range for its result type; the original result is used` | Full のフックの値が結果の型に収まらない |

### パッケージが読み込まれない理由

| ローダーの表示（`Refusal`） | ランチャーの行の表示 | 対処 |
|---|---|---|
| nobody approved it | 「Not allowed · will not run」 | Allowed に切り替える |
| the package changed since it was approved (approved X, this file is Y) | 「The file changed since you allowed it · will not run until allowed again」 | もう一度許可する |
| no origin is recorded: a package of unknown origin is never full-access | 「Cannot run as full-access: This file was put into the mods folder by hand ...」 | Remove してから Add a .yak file… で入れ直す |
| delivered by a server / by a friend: never full-access | 「A mod delivered by a server or a friend cannot be full-access.」 | 許可はできない。自分の PC から入れたコピーを使う |
| another package has the same id | 「Two files have this mod id · neither will run」 | 要らないほうを削除する |
| signature does not verify | （カタログのパッケージのみ） | ローカルのインストールには関係しない |
| malformed package | 「This is not a valid .yak file.」 | パックし直す。`yak verify` を実行する |
| could not start: ... | （読み込み後に失敗） | モジュールがインスタンス化できない、`yoake_init` がトラップした、燃料 5000 万を使い切った、`hook_register` が断られた、のいずれか。「could not start」のあとの文を読む |
| incompatible ABI: ... | （読み込み後に拒否） | `yoake_abi` セクションが無い、壊れている、重複している、またはメジャーバージョンが違う（第 13 章） |
| 「Full-access settings failed their check」のバナー | 「Waits for the settings check above」 | ランチャーの管理ファイルがチェックサムと合わない。プレイヤーが表示された差分を確認する必要がある |

## 12. 上限

| 上限 | 値 |
|---|---|
| `yoake_init` の予算 | 燃料 5000 万と epoch 2000 tick。超えると読み込みに失敗する |
| activation ごとの燃料 | なし（activation のたびに満タン） |
| ウォッチドッグ | 1 activation でゲスト時間が 50 ms を超えるとログに出す。**トラップはしない** |
| 入れ子の深さ | **MOD ごとに** 入れ子 16 activation まで |
| `hook_register` | 1 MOD あたり最大 4096 個。ID 0 は予約。`yoake_init` の中だけ |
| `engine_call` の引数 | 最大 1 MiB |
| 保管される結果（チケット） | 1 activation あたり最大 8 個、合計 16 MiB |
| パスハンドル | 1 activation あたり最大 256 個 |
| モジュールのサイズ | 32 MiB を超える `.wasm` エントリはパッケージの読み込みで拒否される |
| トラップ | その MOD はセッションの間無効になる。ほかの MOD とエンジンは動き続ける |
| 共有（`&self`）の引数 | 読み取り専用。`get` は使え、`set` はハンドルステータス 7（`ReadOnly`）を返す。共有引数に対する型付きの可変呼び出しは `-23` で拒否され、実行されない |

暴走したゲストはウォッチドッグでは止められません。最後の砦はプロセスとプレイヤーです。これがフルアクセスの代償なので、フックは短く保ってください。

## 13. バージョニング: ABI セクション

フルアクセスのゲストはすべて、`yoake_abi` という名前のカスタムセクションに ABI を宣言します。ちょうど 4 バイトで、メジャーバージョンをリトルエンディアンの `u16`、続けてマイナーバージョンをリトルエンディアンの `u16` で置きます。SDK とエンジンは **1.1**（`01 00 01 00`。`1.0` のゲスト `01 00 00 00` も後方互換の古いマイナーとしてそのまま読み込めます）で、`export_guest!` が出力します（SDK の `ABI_MAJOR`、`ABI_MINOR`）。ローダーはこのセクションを、**ゲストのコードを一切実行する前に** モジュールのバイト列から読みます。

| ゲストのセクション | 結果 |
|---|---|
| 無い、長さが違う、重複、壊れている | 拒否 |
| メジャーバージョンが違う | 非互換として拒否 |
| メジャーもマイナーも同じ | 読み込む |
| メジャーが同じでゲストのマイナーが新しい | 読み込むが、MOD がより新しいエンジンを想定している旨をログに出す。エンジンが「未知のバインディング」として断る呼び出しは失敗しうる |
| メジャーが同じでゲストのマイナーが古い | 読み込む（マイナーでの追加は後方互換） |

インポート、バインディング、ステータスの追加はマイナーの変更です。既存のインポートやバインディングの変更や削除、ステータスの意味の変更、ワイヤ形式の変更はメジャーの変更です。バインディング ID は追記のみで再利用されないので、ビルド済みの MOD は同じメジャーの範囲のエンジン更新をまたいで動き続けます。

MOD 自身の `yak.toml` の `version` は別物です。リリースのたびに上げてください。そのたびにプレイヤーは新しいファイルを許可することになります。

## 14. MOD 画面の IME（サンドボックスの `client-full` と F ルートの `f_services`）

フルアクセス MOD には `hooks::IME_PREEDIT`、`hooks::IME_COMMIT`、および `f_services::{ime_set_allowed, ime_set_cursor_area, ime_event}` が用意されています（第 6 章および [`crates/yoake-guest-sdk/examples/f_client_services.rs`](../../crates/yoake-guest-sdk/examples/f_client_services.rs) を参照）。サンドボックス用のサービス番号 201 / 202 をフルアクセスのフック ID として登録しないでください（それらはサンドボックス専用です）。

**サンドボックス型の `client-full` コンポーネント**では、MOD 自身が所有する画面で IME を次のように扱います。

`client-full` の MOD は、`hook::SCREEN_INIT_POST`、`hook::FRAME`、`hook::IME_PREEDIT`、`hook::IME_COMMIT` を購読します。`screen_init_post` にはホストへの書き込み権限がない（サービスを呼ぶと `Err` になる）ため、画面が開いたことだけを覚えておき、`frame` から成功するまで再試行して有効化と候補ウィンドウ位置を設定します。

```rust
use yoake_wasm_sdk::full::client::{ime_set_allowed, ime_set_cursor_area};

fn screen_init_post(&mut self, _screen: et::NativeScreenView) {
    self.pending = true; // MOD 画面を開くたびに IME 状態はリセットされる
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
    // MOD のテキスト欄に `text` を描画する。cursor は UTF-8 バイト位置。
}

fn ime_commit(&mut self, text: String) {
    // 確定文字列を 1 回だけ挿入する。
}
```

`ime_set_allowed` はサービス番号 **201**、`ime_set_cursor_area` は **202** です。矩形 `(x, y, w, h)` は物理ピクセルではなく GUI スケーリング済み座標で渡し、有限数・非負のサイズである必要があります。現在開いている MOD 画面を所有する MOD 以外が呼ぶと `Err` を返します。状態は MOD 画面の開閉ごとにリセットされます。どちらも呼ばない MOD 画面は従来どおり（タイトル上の MOD 画面では有効、ゲーム内では無効）に動き、`[wasm] permissions` に `service.ime` を書く必要もありません（書くとエラーになります）。

**引数の順番の注意**: ジェネレーターはサービス引数をアルファベット順に並べるため、生の WIT インポート `services::ime_set_cursor_area` は `(h, w, x, y)` の順です。順序を直したラッパー `yoake_wasm_sdk::full::client::ime_set_cursor_area(x, y, w, h)` を使ってください。

preedit と commit のフックは MOD 画面が開いている間の購読者にだけ届きます。確定文字列は文字ごとの `screen_char_typed_pre` より先に届くため、二重入力にならないようどちらか一方だけで挿入してください。詳しくは [`docs/guide-ja/10-ime.md`](../guide-ja/10-ime.md) と [`crates/yoake-wasm-sdk/examples/ime/`](../../crates/yoake-wasm-sdk/examples/ime/src/lib.rs) を参照してください。

## 15. ファイル・ネットワーク・ワーカー (FR-F2)

Yoake の MOD は、身内のグループの中だけで使います。だから Forge や Fabric の MOD と同じように、ファイルとネットワークが使えます。変わらない決まりは 2 つです。MOD を入れていないときは何も変わりません。そして、**ゲームのメインスレッドは MOD を待ちません**。ファイルは短くて同期的なので、メインスレッドで使ってかまいません。ネットワークは何秒も待つことがあります。だから**ワーカー**を通します。

この節は、フルアクセス MOD（このガイド）にも、サンドボックスのコンポーネント（`docs/yoake-wasm-dev.md`）にも当てはまります。

### フォルダ

| MOD から見えるパス | 実際のフォルダ | できること |
|---|---|---|
| `/data` | `<ゲームフォルダ>/mod-data/<MOD の id>/`（最初に自動で作ります） | 読み書き |
| `/game` | インスタンスのゲームフォルダ | 読み書き |
| `/host/<ドライブ文字>` | PC のローカルドライブのルート。例: `/host/c`（Unix では `/host/root`） | 読み書き。`[wasm] host_filesystem = true` のときだけ |

保存したいものは `/data` の下にファイルとして置きます。別のストレージ API はありません。

フルアクセス MOD で `std::fs` を使うには、Rust の `std` を **`wasm32-wasip1`** 向けにビルドします。

```sh
rustup target add wasm32-wasip1
rustc --edition 2024 --target wasm32-wasip1 --crate-type cdylib -C panic=abort -o my_mod.wasm src/lib.rs
```

これまでどおり `yoake_init` と `yoake_hook` を書き出し、`env` の関数を使います。`wasi_snapshot_preview1` を何も取り込まない `no_std` の MOD は、今までと同じように読み込まれます。WASI で使えるものは次のとおりです。

- `println!` と `eprintln!` はログの行になります。`wasm mod <id> [stdout]: ...` と `[stderr]: ...` です。サンドボックス側と同じ行です。
- 実時間・単調時計・乱数が使えます。
- **待つことには上限があります。** `thread::sleep`（WASI の `poll_oneoff`）で 2 ms より長く待つと、その MOD は止まります。それより短い待ちはすぐに終わります。ファイルの呼び出しも、フックの時間に入ります。大きなファイルのコピーはワーカーでやってください。

### ワーカー

MOD は **ワーカー** を 1 つ宣言できます。ワーカーは WebAssembly の**コンポーネント**で、エンジンが専用の OS スレッドで動かします。

```toml
[wasm]
interface = "1.7"
world = "full-access"
server = "bin/wasm/my_mod.wasm"
worker = "bin/wasm/my_worker.wasm"   # コンポーネント（world は `worker-mod`）
host_filesystem = false               # 省略できます
```

`server-full` ＋ `worker-mod` のプロジェクトを SDK の `Worker` トレイトと `export_worker!` で作るには `yoake new worker <name>` を実行します。`full-access` コアモジュールと `worker-mod` コンポーネントの組み合わせ例は [`crates/yoake-guest-sdk/examples/f1-f2-demo/`](../../crates/yoake-guest-sdk/examples/f1-f2-demo/src/lib.rs) にあります。

ワーカーでは WASI preview 2 がすべて使えます。上のファイル、`wasi:sockets`（TCP・UDP・名前解決）、`wasi:http` の送信、時間のかかる待ち、本物の時計です。ただし**エンジンのハンドルは持ちません**。MOD とのやり取りは、バイト列のメッセージだけです。WIT は `crates/yoake-wasm/wit-worker/yoake-worker.wit`（パッケージ `yoake:mods@1.7.0`、SDK では `yoake_wasm_sdk::worker::{Worker, send, recv, try_recv}` と `export_worker!`）にあります。追加だけのマイナー更新で、1.0〜1.2 のものは何も変えていません。

| 側 | 呼び出し |
|---|---|
| MOD（フルアクセスのコアモジュール） | `yoake_worker_send(ptr, len) -> i32`、`yoake_worker_recv(ptr, cap) -> i32`（モジュール `env`）。SDK: `yoake_guest_sdk::worker::{send, recv}` |
| MOD（サンドボックスのコンポーネント） | `worker-send(list<u8>) -> result<_, string>`、`worker-recv() -> option<list<u8>>`（`yoake:mods/worker@1.6.0`） |
| ワーカーのコンポーネント | `host-send`、`host-recv`（メッセージが来るか、ワーカーが止まるまで待つ）、`host-try-recv` を取り込む（`yoake:mods/worker-host@1.6.0`）。`run: func()` を書き出す（`yoake:mods/worker-run@1.6.0`） |

決まり:

- 行きと帰りに、それぞれキューがあります。最大 **1024 件・16 MiB** です。いっぱいのとき、送信はエラーを返します（`queue-full`。コア ABI では `-1`）。待ちません。16 MiB を超える 1 件は `too-large`（`-3`）です。
- `yoake_worker_send` の戻り値は、`0` 成功、`-1` キューがいっぱい、`-2` ワーカーなし（宣言がない、または止まった）、`-3` 大きすぎる、`-4` ポインタが不正です。
- `yoake_worker_recv` の戻り値は、長さ（0 より大きい）、`0` なし、`-n` 次のメッセージに `n` バイト必要（メッセージはキューに残る）、`i32::MIN` ワーカーなし、`i32::MIN + 1` ポインタ不正です。コア ABI では、空のメッセージは「なし」と同じ扱いです。
- `run` はワーカーのスレッドで 1 回だけ呼ばれます。戻るとワーカーは終わります。トラップ・パニック・`exit` で止まるのはワーカーだけです。`wasm mod <id> worker: ended, the worker is disabled: ...` というログが出ます。MOD は動き続け、送信は `closed`（`-2`）になります。
- MOD をアンロードしたとき（ホットリロード、ワールドを出たとき）と、ゲームを終了したときに、ワーカーも止まります。キューを閉じ、待っている `host-recv` は `none` を返し、ワーカーのコードは約 10 ms 以内に止められます。ソケットの中で待っているワーカーは、0.5 秒待ってそのまま手放します。
- クライアントとサーバーの両方のモジュールがあるパッケージは、インスタンスごとにワーカーが 1 つ動きます。
- ワーカーの標準出力と標準エラーは `wasm mod <id> worker [stdout]: ...` というログになります。

例（ワーカー側、Rust と `wit-bindgen`）: `crates/yoake-wasm-sdk/examples/f2-io`（`--features worker`）。

### メインのインスタンスのネットワーク

サンドボックスのコンポーネントのメインのインスタンスには、`wasi:sockets` と `wasi:http` をつなぎますが、使うことは断ります。接続やリクエストはアクセス拒否のエラーを返します。待つ呼び出しはゲームのメインスレッドを止めてしまうからです。ネットワークはワーカーで行ってください。（`Sandbox::set_main_network(true)` で解除できます。ツールとテスト用で、ゲームは使いません。）

### サービスの権限ゲートはなくなりました

サンドボックスの MOD のサービス呼び出し（`config`・`permissions`・`storage`）は、`[wasm] permissions` がなくても使えます。古い MOD が読み込めるように、この項目は今も読み取ります。`service.commands`・`service.chat`・`service.gpu` の意味は変わりません。`service.http` はマニフェストでは受け付けていましたが、通信の部分はつながっていませんでした。HTTP はワーカーの `wasi:http` を使ってください。

## 16. レベルとワールド生成（F3-8b）

サーバー側のインジェクトフックが 4 つ用意されています（すべて `side = "server"`、ID は `inject-sites.json`、購読する MOD がなければノーオペレーションです）。

| ID | 引数 | 呼ばれるタイミング |
| --- | --- | --- |
| `level::before_open` | `&mut Game`, レベル ID | まだ開いていないレベルをフォルダから読み込む直前 |
| `level::after_open` | `&mut Game`, レベル ID | レベルが開き、MOD が `LevelEvent.Load` を受け取ったあと |
| `level::before_close` | `&mut Game`, レベル ID | `unload`・`delete`・`regenerate` で閉じる直前（チャンクとファイルがまだ残っている間） |
| `worldgen::generate_chunk` | レベル ID, `cx`, `cz`, `&mut ModChunk` | MOD ジェネレーター（`yoake:mod`）を使うレベルのチャンクごとに、ジェネレーターが埋める直前に 1 回 |

`worldgen::generate_chunk` はバニラの 3 ディメンションや `flat` / `debug` / `noise` のレベルでは呼ばれないため、バニラ地形には一切影響しません。フックから `ModChunk` のバインディング（`stubs::mod_generator` の `set_block`、`fill_layer`、`block`、`set_biome`、`mark_failed`）で空のチャンクに書き込めますが、そのあとにレベルのジェネレーターがチャンクを埋めます。地形そのものを置き換えたい場合はジェネレーター自体を登録してください（サンドボックス WebAssembly MOD の `generator_register`、第 13 章 [`13-dimensions.md`](../guide-ja/13-dimensions.md)）。チャンク生成ワーカースレッドで動くため処理は短く保ってください。呼び出し上限を超えると下の安全性ルールによりそのチャンクは未生成扱いになります。

WebAssembly サービス側は後方互換の列挙型を維持するため、`current_level(player)`（サービス 235）がプレイヤーの現在いる実際のレベル ID（文字列）を返します。

ジェネレーターの安全性: MOD ジェネレーターが応答できなかった場合（モジュールがビジー、トラップ、呼び出し上限超過）、約 1 ティック後に 1 回だけ再試行します。それも失敗するとチャンクはジェネレーターが書いた状態のまま `not_generated`（未生成）とマークされ、保存されません（次回読み込み時に再生成されます）。誰かがそのチャンク内のブロックを書き換えるとマークが外れ、建築した内容は保存されます。

## 17. カスタム Mob と brain オプション (F4M-4b)

カスタム Mob（`data/<id>/horizon/entity_type/<name>.json`、`"template": "yoake:mob"`）は、素の `ai.goals` / `ai.targets` だけでなく、**brain オプション**（`ai.memories`、`ai.sensors`、`ai.activities`、`ai.schedule`、`ai.default_activity`）を宣言できます。これらの brain フィールドを持たない Mob では、従来のゴールがそのまま変わらぬ挙動で動きます。

### メモリ (`ai.memories`)

1 型につき最大 64 個の型付きメモリスロットを宣言できます。

| `type` | JSON の `default` | 備考 |
|---|---|---|
| `bool` | `true` / `false` | |
| `i32` | 整数 | |
| `f32` | 有限の浮動小数 | |
| `string` | 文字列 | 最大 256 バイト |
| `vec3` | `[x, y, z]` | 有限の浮動小数 3 要素 |
| `block_pos` | `[x, y, z]` | 整数のブロック座標 |
| `entity` | — | 実行時の Mob ID またはプレイヤー UUID（JSON リテラルなし） |

各定義には `"default"`、`"persistent": true|false`（エンティティ NBT の `yoake.memories.<name>` に保存され、別の MOD バージョンが残した未知キーは保持されます）、`"ttl": 1..=72000`（書き込み時に明示しない場合のデフォルト期限 tick 数）を指定できます。

### センサー (`ai.sensors`)

1 型につき最大 32 個の宣言型センサーを置けます（`interval` は 1〜2400 tick、既定 20、任意の `memory` と `ttl`）。

- `nearest_players`（`range`、`must_see`、`ignore_creative`）→ `entity` を書き込みます（既定キー `nearest_player`）。
- `nearest_living`（`types`、`range`、`must_see`）→ `entity` を書き込みます（既定キー `nearest_living`）。
- `hurt_by`（`max_age`）→ `entity` または `bool` を書き込みます（既定キー `hurt_by`）。
- `blocks`（`tag` および／または `blocks`、`radius` 1〜32、`vertical_radius` 0〜16）→ `block_pos` または `vec3` を書き込みます（既定キー `nearest_block`）。
- `light`（`kind`: `combined` | `sky` | `block`）→ `i32`・`f32`・`bool` のいずれかを書き込みます（既定キー `light_level`）。

### アクティビティとスケジュール (`ai.activities`, `ai.schedule`, `ai.default_activity`)

- アクティビティ（`priority` 0〜32・既定 5・**小さい数値が優先**、`core`: 真偽値）には、条件 `when` と `goals` / `targets` を置けます（トップレベルのゴールに `"activity": "<name>"` を付ける書き方も可能です）。
- `core: true` のアクティビティは、`when` が満たされているあいだ、現在選ばれている非 core アクティビティと並行して動きます。
- `when` の条件はメモリ 1 つを判定します（`present: bool`、`eq`、`ne`、および `i32`/`f32` に対する数値比較 `gt`/`gte`/`lt`/`lte`）。
- 同じ `priority` の場合、`when` 条件で有効になるアクティビティがスケジュールのみ（`when` が空）のアクティビティより優先されます。
- `ai.schedule` には `{"time": 0..24000, "activity": "<name>"}` を並べます（`day_time % 24000` で判定）。どの条件付き／スケジュール済みアクティビティも選ばれない場合は `ai.default_activity`（なければ存在する場合の `"idle"`）が使われます。

### スクリプトからのアクセス (`service.mobs`)

`[wasm] permissions = ["service.mobs"]`（`crates/yoake-wasm-sdk`、`full::server::mobs`）を持つサーバーモジュールは、宣言された brain メモリを読み書きできます。

- `mobs::memory_get(entity, key) -> Result<Option<MobMemoryValue>, String>`（サービス 287: `mob-memory-get`）。
- `mobs::memory_set(entity, key, value: Option<MobMemoryValue>, ttl_ticks: Option<u32>) -> Result<(), String>`（サービス 288: `mob-memory-set`。`value = None` で消去、`ttl_ticks = None` でメモリの既定 `ttl` または無期限）。
- 値コンストラクタ: `mobs::memory_bool`、`memory_i32`、`memory_f32`、`memory_string`、`memory_vec3`、`memory_block_pos`、`memory_entity_mob`、`memory_entity_player`。
- 例: `crates/yoake-wasm-sdk/examples/fox_ai` と `tools/fixtures/cinder_fox/content/data/sample_mobs/horizon/entity_type/cinder_fox.json`。

### 実行時エンティティテクスチャ (`service.mobs`, F4M-7)

`client-full` ワールドのクライアントモジュール（`[wasm] world = "client-full"`、`client = "bin/wasm/..."`、`permissions = ["service.mobs"]`）は、`yoake_wasm_sdk::full::client` を通じてカスタム Mob 用の RGBA8 テクスチャを実行時に確保・描画・バインドできます。すべて各クライアント上で完結し、ネットワークには送信されません（一般的には `hook::SYNCED_CHANGED_CLIENT` から更新します）。

- `client::mob_texture_create(width, height) -> Result<u32, String>`（サービス 289: `mob-texture-create`）: エンティティ・アトラス内の予約領域 `1024x1024`（`horizon/mob-textures`）に `width` x `height`（各辺 `1..=1024`）のスロットを確保します。1 MOD あたり同時に保持できるテクスチャは最大 **64** 個です。
- `client::mob_texture_write(id, origin, size, rgba) -> Result<(), String>`（サービス 290: `mob-texture-write`）: テクスチャ `id` の矩形 `[origin.0, origin.1, size.0, size.1]` に RGBA8 ピクセルを書き込みます（全体更新・部分更新の両対応、1 回あたり最大 **4 MiB**）。
- `client::mob_texture_bind(entity, slot, id: Option<u32>) -> Result<(), String>`（サービス 291: `mob-texture-bind`）: カスタム Mob `entity` の `slot` にテクスチャ `id` をバインドします（`None` でパック元のテクスチャへ戻します）。
  - `""` または `"default"`: モデルの `default` テクスチャ（およびそれを継承する `model.layers` のレイヤー）を差し替えます。
  - `<layer_name>`: `model.layers` 内の指定名のレイヤーを差し替えます。
  - `"glowmask"`: 自動検出された `<texture>_glowmask` 発光パスを差し替えます。
  - `<bone_name>` または `"bone:<bone_name>"`: 本体パスのうち、そのボーンのクワッドだけを差し替えます。
- **引数の順番の注意:** ジェネレーターは WIT 引数をアルファベット順に並べるため、生のインポートは `services::mob_texture_create(height, width)`、`services::mob_texture_write(id, origin, rgba, size)`、`services::mob_texture_bind(entity, id, slot)` になります。`(width, height)`、`(id, origin, size, rgba)`、`(entity, slot, id)` の順を保つ `yoake_wasm_sdk::full::client::mob_texture_*` ラッパーを使ってください。
- **寿命とリロード:** 作成済みのテクスチャとバインドは、リソースパックの再読み込み（アトラス再構築）やシェーダーパックの切り替えを跨いで維持されます。MOD のアンロード／再読み込み時には、その MOD が所有するテクスチャとバインドが自動的に解放・解除されます。
- 例: `crates/yoake-wasm-sdk/examples/fox_texture`（`fox_ai` が `mob_hurt` で `warmth` を更新した際に Cinder Fox のテクスチャを塗り替えます）。

### フルアクセスの `custom_mobs`（ルート F、F4M-11）

フルアクセスのコアモジュールからは、`yoake_guest_sdk::custom_mobs`（`stubs::custom_mobs` のラッパー）でカスタム Mob を制御・描画できます。

- **フック（`hooks::*`）**:
  - サーバー側のゴール・ライフサイクル: `MOB_GOAL_CAN_USE`、`MOB_GOAL_CAN_CONTINUE`、`MOB_GOAL_START`、`MOB_GOAL_TICK`、`MOB_GOAL_STOP`（`custom_mobs::register_goal_hooks(r, 0)` でまとめて登録可能）、`MOB_TICK`、`MOB_SPAWN_CHECK`、`MOB_SPAWNED`、`MOB_HURT`、`MOB_DIED`、`MOB_INTERACT`、`MOB_ANIM_EVENT`、`MOB_LOAD`、`MOB_SAVE`、`MOB_TAMED`、`MOB_BRED`、`MOB_SYNCED_CHANGED_SERVER`。
  - クライアント側の描画ステージ・イベント: `CUSTOM_MOB_BEFORE_POSE`、`CUSTOM_MOB_AFTER_POSE`、`CUSTOM_MOB_AFTER_VERTICES`（`custom_mobs::register_draw_stage_hooks(r, 0)` で登録）、`MOB_POSE`、`MOB_ANIM_EVENT_CLIENT`、`MOB_SYNCED_CHANGED_CLIENT`。
- **イベントの取得と応答**: `custom_mobs::hook_event()`（描画ステージでは `draw_event()`）でイベントを読み、`reply_bool`・`reply_hurt`・`reply_interact`・`reply_save`・`reply_pose`・`draw_cancel` で応答します。
- **操作（`CustomMob`）**: `CustomMob::new(mob_id)` から `.info()`、`.synced()` / `.set_synced()`、`.trigger_anim()`、`.set_controller_state()`、`.hold_controller_state()` / `.release_controller_state()`、`.navigate_to()` / `.navigate_stop()`、`.look_at()`、`.jump()`、`.set_target()`、`.attack()`、`.impulse()`、`.attribute()` / `.set_attribute_base()` / `.add_attribute_modifier()` / `.remove_attribute_modifier()`、`.set_hitbox_enabled()`、`.data()` / `.put_data()`、`.seat_set()`、`.memory()` / `.set_memory()`、`.locator()`、`.bind_texture()`、`.claim_control()`、`.despawn()` を呼べます。
- **ゴール登録とボーン姿勢**: `custom_mobs::register_goal(name)`、`spawn_queued(...)`、`texture_create` / `texture_write`、姿勢フック内の `pose_bone_count` / `pose_get` / `pose_set` / `pose_write` を備えています。
- **サンプル**: [`crates/yoake-guest-sdk/examples/fox_ai_full.rs`](../../crates/yoake-guest-sdk/examples/fox_ai_full.rs)（`fox_ai` のルート F 版）。

### カスタム Mob のデバッグ表示とホットリロード (F4M-12, F4M-13)

**デバッグ表示**はプレイヤー自身のクライアントだけのもので、サーバーには何も送りません。オフの間は何のコストもかかりません。

- `/yoake mobdebug [on|off] [hitboxes] [bones] [anim] [ai]`: 何も付けないと全部をオン／オフの切り替え、`on` だけだと 4 つすべてオン、`off` で表示オフ、パーツ名だけを付けるとそのパーツだけがオンになります。F3+M も同じ切り替えです（カスタム Mob の型があるワールドだけ）。
  - `hitboxes`: 型の `hitboxes` の当たり判定・選択パーツを、名前とダメージ倍率つきの箱で表示します。緑は被ダメージと選択の両方、赤は被ダメージのみ、青は選択のみ、灰色は無効にしているパーツ（`set_hitbox_enabled`）です。
  - `bones`: 骨格を、各ボーンの原点から子ボーンへの線（体の上に重ねて、描画中のポーズのまま）で、ボーン名、`locators`（水色の小さな箱と名前）、`seats`（橙色の箱）とあわせて表示します。
  - `anim`: コントローラーのレイヤーごとに、現在のステートと最初のクロックの時間、そして同期値（`synced`）を表示します。
  - `ai`: ターゲットへの矢印、ナビゲーターの経路（次のノードから）、実行中のゴールとターゲットゴール、ブレインの現在のアクティビティを表示します。
  - 見えている Mob（近い順に 48 体、48 ブロック以内）には、型 ID・Mob ID・体力の見出し行も付きます。
- `/yoake mobs`: 読み込まれているカスタム型と、その生きている数・描画している数を表示します。最初の呼び出しでコスト計測が始まり、数秒後にもう一度実行すると、型ごとに 1 フレームあたりの姿勢計算と頂点生成の平均マイクロ秒が出ます。続けて `/horizon custommobs stats` の経路予算の行が出ます。`/yoake mobs stop` で計測を止めます。（型ごとのサーバーのティックコストはこのレポートにはまだ入っていません。今カウントしているサーバー側の型別コストは経路探索だけです。）
- コマンドは、どのサーバーでも、チャットで入力したプレイヤー本人のゲームが答えます。Horizon 独自コマンドを有効にしている場合（`--pregen`）は、ゲームマスター向けにコマンドツリーにも載るので、タブ補完や制御ソケットの `command` から使えます。

**ホットリロード（開発用）**は、開発者モード（`YOAKE_WASM_UNSIGNED=1`）と、`SIGNATURE` のないパッケージフォルダ（`--dev-mod` のフォルダ、または署名なしでインストールしたパッケージ）が必要です。署名つきパッケージはリロードされません。

- **アセット (R1)**: ゲームは 1 秒に 1 回、各カスタム型のファイル（JSON、geo／bbmodel、アニメーション、コントローラー、テクスチャ、グローマスク）を調べます。ファイルが丸 1 秒変わらずにいたとき（エディターや書き出しがまだ書き込み中かもしれないため）に変更として扱います。その型を起動時と同じローダーで読み直し、動いているその型の Mob は、次のゲームティック（フレームとフレームの間）で新しいモデル・アニメーション・レイヤーに切り替わります。**アニメーションの位相は保たれます**。位相はサーバーが固定したクロックが持っているので、歩いているキツネは歩幅の同じ位置から歩き続けます。テクスチャの変更は、代わりにリソースの再読み込み（F3+T と同じ）を行います。
- **定義 (R2)**: `/yoake mob reload [型]` は、指定した型（名前なしならリロードできるすべての型）を、待たずにすぐ読み直します。生きている Mob には新しい数値（`max_health`、`movement_speed`、属性。Mob 側で変えていない値はファイルに追従し、ケガをした Mob はケガをしたままです）、`ai` が変わったときは作り直したゴールリスト、新しい当たり判定パーツが反映されます。リロードの直後にスポーンした Mob も 1 ティック以内に切り替わります。
- **壊れたファイルは古い版のまま**: ログに、ファイルと理由を示す警告が 1 回出て（`Horizon custom-mob reload: <型> not reloaded, the old version keeps running ...`）、直前に読めた型で動き続けます。ファイルを直して保存し直してください。
- **ゲームを動かしたままではリロードできないもの**: `synced` の値の変更、コントローラーのレイヤー・ステート・トリガーの変更（ネットワーク形式が依存するもの）。メッセージつきで拒否されるので、ゲームを再起動してください。型の削除と新しい型の追加も再起動が必要です。
- **サーバーモジュール (R3)**: `YOAKE_WASM_SERVER_RELOAD=1` も指定すると、署名なしサーバーモジュールの `.wasm` ファイルを監視し、新しいビルドをサーバーティックで差し替えます。Mob はエンジンが持っているので状態は残り、ゲスト自身のメモリ（変数に持っていたもの）はやり直しになります。読み込めないビルドは、警告を 1 回出して古いインスタンスを動かし続けます。このスイッチがなければ、サーバーモジュールはこれまでどおり 1 回だけ読み込まれます。

## 18. データだけで足せるもの: エンチャント・ダメージ種別・ゲームルール・流体・整数のブロック状態 (REM-16)

`registries` 権限をもつ `.yak`（または `.hmod`）は、モジュールのコードなしで、`data/<id>/horizon/<種類>/<パス>.json` にこれらを書いて足せます（ID は `<id>:<パス>`）。項目の全リストは [docs/mods.md](../mods.md#enchantments-damage-types-game-rules-and-fluids) にあります。これらのファイルがない MOD は、ゲームを何も変えません。

| 種類 | ファイル | 何になるか |
| --- | --- | --- |
| `enchantment` | バニラのエンチャント JSON | コンテンツルートの `data/<id>/enchantment/<パス>.json`。レジストリ（データごとクライアントへ同期）、`/enchant`、エンチャントテーブル、金床、ルートの関数、アイテムの `enchantments` コンポーネントで使える |
| `damage_type` | バニラのダメージ種別 JSON | `data/<id>/damage_type/<パス>.json`、ダメージカタログ、コマンドの ID 候補（`/damage`） |
| `game_rule` | `{"type": "bool"/"int", "default", "min", "max", "category"}` | ルール `<id>:<パス>`。`/gamerule`、ルール画面、管理 API で読み書きでき、ワールドと一緒に保存される |
| `fluid` | `flowing_id`、`still_block`、`flowing_block`、`bucket`、`still_texture`、`flowing_texture`、`tint`、`viscosity`、`tick_rate`、`light`、`can_drown`、`motion` | ワールドの流体レジストリの 1 ファミリー。モジュールの `fluids` 登録と同じもの |

```json
// data/ruby/horizon/enchantment/keen.json
{ "description": { "translate": "enchantment.ruby.keen" },
  "supported_items": "#minecraft:enchantable/sharp_weapon",
  "weight": 5, "max_level": 3,
  "min_cost": { "base": 5, "per_level_above_first": 8 },
  "max_cost": { "base": 25, "per_level_above_first": 8 },
  "anvil_cost": 2, "slots": ["mainhand"] }

// data/ruby/horizon/damage_type/shatter.json
{ "message_id": "shatter", "scaling": "when_caused_by_living_non_player", "exhaustion": 0.1, "effects": "freezing" }

// data/ruby/horizon/game_rule/gem_rate.json
{ "type": "int", "default": 3, "min": 0, "max": 10, "category": "misc" }

// data/ruby/horizon/fluid/oil.json （ブロック ruby:oil と ruby:flowing_oil、アイテム ruby:oil_bucket、テクスチャ block/oil_still.png と block/oil_flow.png）
{ "tint": 4473924, "viscosity": 4, "tick_rate": 10, "motion": "water" }
```

モジュールからは、`world-control` の `set-game-rule` に、ゲームが登録したルールの名前空間つきの名前（`ruby:gem_rate`）を渡せます（知らない名前や `minecraft:...` はこれまでどおり拒否されます）。`/damage` はダメージ種別の ID で `execute-command` から使えます。ルールと流体は起動時に登録するので、増減には再起動が必要です。既存の流体は、リロードで `motion`・`tick_rate`・`viscosity` を更新できますが、ほかの項目の変更にも再起動が必要です。ファイルの流体とモジュールの `fluids` 登録は 1 つのレジストリ（最大 125 ファミリー）に入り、2 つのファミリーが同じ名前を使うと、その流体の名前つきで拒否されます。

ブロックの状態プロパティには、`boolean`・`enum` のほかに `int` が加わりました: `"state_properties": { "age": { "type": "int", "min": 0, "max": 7 } }`（バニラの `IntegerProperty`。`min` は 0 以上で `max` より小さく、値は最大 256 個、昇順）。値は `default_state` と `random_ticks` では 10 進のテキスト（`"default_state": { "age": "0" }`）で、モデルのスキーマには整数の範囲として載るので、ブロック状態ファイルでバニラの作物と同じように `age=3` で選べます。ブロックはこれまでどおりバニラの*テンプレート*のコピーですが、`"template": "yoake:cube"` で素の立方体（石の 1 つだけの状態。性質とファイルはブロック自身の `properties`・ブロック状態・モデル・ルートテーブルで置き換わる）が作れます。バニラの行をまったく持たないブロックはまだ使えません（レポート FR-REM16 を参照）。

## 19. 自分のブロック・アイテム・メニュー・レシピ種別に振る舞いを付ける (REM-15)

Forge の mod は `Block`・`Item`・`MenuType` を継承して振る舞いを書きます。サーバーモジュールは **behaviour（振る舞い）** で同じことをします。
`init` の中で、パッケージ自身のコンテンツのどれに対して、どのフックを実装しているかをゲームに伝えます。以後ゲームは、
その対象に限って、モジュールの `handle-call` エクスポートを呼びます。それ以外のブロック・アイテム・メニューのコストは今までと
同じです（何も見つからない表引きが 1 回。どのモジュールも何も登録しなければ、それすらありません）。こうした mod の
入っていないゲームは今まで通りに動きます。

```rust
use yoake_wasm_sdk::full::behaviors::{self, Behaviors, BlockAnswer, BlockUse, Interaction, block};
use yoake_wasm_sdk::full::server::{ServerMod, ext, services, types::BlockPos};

#[derive(Default)]
struct Guest;

impl Behaviors for Guest {
    fn on_block_use(&mut self, call: &BlockUse) -> BlockAnswer {
        // フックの中ではサービスが使える: クリックされたブロックの `lit` を反転する
        let pos = BlockPos { x: call.block.pos[0], y: call.block.pos[1], z: call.block.pos[2] };
        // ... block_property / set_block_property ...
        BlockAnswer::result(Interaction::Success)
    }
}

impl ServerMod for Guest {
    fn subscriptions() -> &'static [&'static str] { &[] }
    fn init(&mut self) {
        ext::register_behavior("block", "ruby:lamp", &[block::USE]).ok();
    }
    fn handle_call(&mut self, _caller: String, method: String, payload: Vec<u8>) -> Result<Vec<u8>, String> {
        behaviors::handle(self, &method, &payload).unwrap_or_else(|| Err(format!("unknown method {method}")))
    }
}
yoake_wasm_sdk::export_server_full!(Guest);
```

`ext::register_behavior(kind, id, hooks)` はサービス `behavior-register`（292）です。サンプルは
`crates/yoake-wasm-sdk/examples/ruby-behaviors`（使うと切り替わるランプ、杖、金庫メニュー、合金レシピ種別。コンテンツファイル付き）、
ホスト側のテストは `crates/horizon-client/src/wasm_mods/behaviors/tests.rs` です。

**登録の決まり。** `id` はパッケージ自身のもの（`<パッケージ id>:<パス>`）に限ります。`kind` は `block`・`item`・`menu`・`recipe`。
フック名は下の表のもの。同じ対象を 2 つのパッケージが取ることはできません。ゲームは表をゲーム起動時に 1 度だけ読み、ワールド開始のたびに
`init` がもう一度走ります。同じ登録は再び受け付けられ、ゲーム起動後に新しい対象を足したり別のフックの組にしたりするとメッセージ付きで
断られます（変えるにはゲームを再起動）。1 パッケージ・1 種別につき 256 個までです。

| 種別 | フック | エンジンが呼ぶとき | フックが持つゲームへの手 |
| --- | --- | --- | --- |
| `block` | `use` | プレイヤーがブロックを右クリックした（サーバー側。答えは `InteractionResult` と同じ `Pass`・`Success`・`Consume`・`Fail`） | サービス（`block-property`・`set-block-property`・`give-items` など） |
| | `placed`、`removed` | この id のブロックが `Level.setBlock` で現れた／消えた（状態だけの変化はどちらでもない） | なし。コマンドで答える |
| | `neighbor-changed`、`random-tick`、`scheduled-tick` | レベルの tick（`random-tick` を持つとランダム tick を受ける） | なし。コマンドで答える（その場で適用） |
| | `entity-inside`、`step-on` | エンティティがブロックに触れた | なし。コマンドで答える |
| `item` | `use`、`use-on-block`、`use-on-entity` | 空中で／ブロックに（ブロックがクリックを取らなかったとき）／エンティティに使った | サービス |
| | `finish-using` | 食べる・飲むが終わった。別のスタックを残せる | サービス |
| | `inventory-tick` | プレイヤーのインベントリにある間、毎 tick | サービス |
| `menu` | `create` | プレイヤーがメニュー種別を開いた（`open_menu`）: 大きさと最初の中身を答える | サービス |
| | `may-place`、`clicked` | スロットにスタックを入れようとした／スロットのクリックが適用された（スロット・ボタン・クリックの種類・その後の中身。答えでスロットを書き換えられる） | なし |
| | `quick-move` | シフトクリック: 答えで「ゲスト側が移動を引き取った」かどうかとスロットの書き換えを返す | サービス |
| | `still-valid`、`closed` | 開いている間毎 tick／閉じた（そのときの中身） | サービス |
| `recipe` | `describe`、`decode`、`matches`、`assemble`、`display` | 下を参照 | なし |

自分のゲームへの手を持たないフック（レベルが回す tick、変化の観測、ブロックに触れたエンティティ）は、書き込みサービスを呼べません。
代わりに **コマンドで答え**、エンジンが検査して適用します: `set-property`（位置のブロックのプロパティ）、`schedule-tick`、`play-sound`。
1 回の答えに 64 個まで、対象から 32 ブロック以内。レベルの tick のコマンドはその tick で、`placed`・`removed`・`entity-inside`・`step-on`
のコマンドはそのレベルの次の tick まで待ち、プロパティの設定だけができます。

**どこで動くか。** ほかのサーバーモジュールのフックと同じく、サーバーの上です。専用サーバー、またはワールドを LAN に公開したゲーム（その内蔵サーバー）。
クライアント側の予測（サーバーに参加したゲームの中）では何も呼びません。LAN に公開していない単独プレイの窓も（サーバーを持たないので）呼びません。決めるのはサーバーです。自分のサービスが原因で
同じモジュールにフックが戻ってきた場合（ブロックを置いたら隣の更新が同じモジュールに来た、など）は、モジュールがまだ使用中なので
そのフックは捨てられ、数えられ、1 回だけログに出ます。そのときは対象はフックなしのまま振る舞います。呼び出し 1 回あたりの上限を超えた
ゲストはほかのフックと同じように止められ、失敗した呼び出しや検査を通らない答えは何も変えません。呼び出しはモジュールの通常の上限に
数えられます。

**位置で指すブロックエンティティ。** サービス `block-entity-read-at`（293）と `block-entity-patch-at`（294）、SDK の
`ext::block_entity_read`・`block_entity_merge`・`block_entity_replace` は、位置にあるブロックエンティティの保存データを読み書きします。
バニラのチェストでも mod のブロックエンティティでも、`/data get block`・`/data merge block` と同じです。データはネットワーク形式の NBT
コンパウンド 1 つ（`yoake_wasm_sdk::nbt::Compound`）で 64 KiB まで。エンティティのデータにマージされ（mod のブロックエンティティに限り置き換えも可）、
チャンクは次に保存され、クライアントには更新タグが届きます。mod ブロックエンティティのデータ（`nbt`・`items`・`data`）はもともとチャンクと一緒に保存・読み込みされます。
このサービスは tick フックの外からそれに触れるための手段です。

**レシピ種別。** `recipe` の対象は、レシピファイルが名前で指せるレシピ種別です（`"type": "ruby:alloy", "version": 1`）。ゲストは
`describe`（レシピが持つバージョンと、独自の id・アイコンのレシピブック分類）に答え、`decode`（レシピの JSON をゲームが持っておくデータ、64 KiB まで、に変える。
なければデータは JSON テキストそのもの）に答えてもよく、`matches`（クラフトの並び）・`assemble`（できるもの）・レシピブック用の `display` に答えます。
`behaviors::json_fields` と `json_strings` で JSON ライブラリなしにレシピの JSON を読めます。ゲームへの手を持つフックから使えるサービスが 2 つあります。
`recipe-match`（295、`ext::recipe_match`）は「この並びからこの種別でできるもの」をゲームに尋ね、`recipe-data`（296、`ext::recipe_data`）は読み込み済みの
その種別のレシピを、種別が持っているデータ付きで並べます。フックの実行中のパッケージは呼び出せないので、`recipe-match` は種別を定義した
パッケージ自身には（その旨のエラーで）断ります。同じパッケージのメニューは `recipe-data` のデータを自分の `on_recipe_matches` で照合し（サンプルの金庫は
`quick-move` でそうしています）、別のパッケージのメニューは `recipe-match` を使います。どちらも作ったり消したりはしません。

| 上限 | |
| --- | --- |
| 1 パッケージ・1 種別の対象 | 256 |
| 1 回の答えのコマンドと届く範囲 | 64 個、32 ブロック |
| ブロックエンティティのデータ、レシピのデータ | 64 KiB |
| メニュー | 256 スロットまで。`create` は開くたびに 1 回、サーバーだけ |

この章にないもの: バニラのテンプレート行を持たないブロック（18 章）、エンティティの振る舞い、内蔵の流動とバケツを超える流体の振る舞い、フックで書く能力（`capability_providers`: エネルギー・アイテム・流体のハンドラ）。
カタログの全体フック（全ブロックの `random-tick`、`use-item-on` など）は今まで通りで、自分のものでなく全部を相手にしたい mod 向けです。詳しくは報告書 FR-REM15。

## 要確認（確かめられていないこと）

1. **クライアント側モジュール** はマージ済み（MX3CL1）で、ローダーを通したテストはしてあります。ただし、実際に動いているゲームのクライアントでは試していません。本物のウィンドウや HUD に対して、クライアント側モジュールを動かしたことはありません。
2. **プレイヤーが `yoake:` の行をどこで見られるか。** stderr と `log` クレートに出力されますが、ランチャーがゲームの stderr をどこに保存するかは見つけられなかったため、どのファイルを開くかは書いていません。
3. **実際のゲームでの動作。** ゲームは一度も起動していません。ひな形とサンプルは、本物のローダーと仮の `Game` ルート（パス `Game.experience` に本物の `Experience` を置いたもの）で動かしました。本物の `Game` に対して同じ SDK サンプルを動かすテストは `crates/horizon-client/src/game/yoake2_examples_tests.rs` にありますが、`HORIZON_TEST_RESOURCES` がないと無視されます。
4. **「許可したあとにゲームを起動（再起動）する」** はコードからの推論です（ランチャーはゲームを起動する前にリストを書き、ゲームは最初のワールド開始時に 1 度だけ読む）。動作中のランチャーでは確認していません。
5. **呼び出し箇所フック**（台帳の `kind = callsite`）: 台帳とブリッジでは関数フックと同じように並んでいて、`docs/yoake2/HOOK-SLICE-BRIDGE.md` には、ゲストから見えるのは引数 0 のハンドルで、ほかの引数は見えないとあります。このガイドのために呼び出し箇所フックを動かしてはいません。
6. **`yoake new full-access` の実ゲーム E2E 動作**: `full-access` テンプレートと `f1-f2-demo` に対する `yoake build` / `yoake check` は `tools/verify_f5_kit.py` で、`f_client_services` と `fox_ai_full` は `crates/yoake-guest-sdk/tests/guest_runtime.rs` で検証済みですが、ドキュメント更新のこの作業では GUI ゲーム本体を起動した確認は行っていません。
7. **`build.sh` は Windows の Git Bash でしか実行していません。** Linux と macOS は試していません。この Windows（GNU ツールチェーン）のマシンでは、パックツールのコンパイルに MinGW（`dlltool`、`as`）のフォルダを `PATH` に通す必要がありました。
8. **フック ID は安定した公開一覧ではありません。** 台帳には `tombstone` のエントリやブリッジされていない ID があります。ブリッジ済みのフックが 201 個というのはこのコミット時点の数です。
9. **フック 1 回あたり 110〜150 ns** は S2 スパイクのレポート（`MX3S2.md`）にある、1 台のマシンでの測定値です。桁の目安として読んでください。
