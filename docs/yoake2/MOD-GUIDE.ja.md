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

Yoake のソース一式（SDK とパックツールが入っています）と、それが指定するツールチェーンが必要です。

- ソース一式のルートにある `rust-toolchain.toml` が **rustc 1.100.0** と、ターゲット `wasm32-unknown-unknown`・`wasm32-wasip2` を指定しています。ソース一式の中では rustup が自動で選びます。外では `rustc +1.100.0` と書いてください（ひな形のスクリプトはそうしていて、`YOAKE_TOOLCHAIN` で変えられます）。
- MOD は素の `rustc` で **`wasm32-wasip2`** 向けにビルドします（Cargo は不要、コンポーネントの包みもなし）。出力は WASI インポートを持たないコアモジュールです。
- **GNU ツールチェーンの Windows**: パックツール（`cargo-horizon-mod`）のコンパイルでは `dlltool` とアセンブラが `PATH` に必要になるクレートを使います。MOD 自体は不要で、`rustc` と同梱の `rust-lld` だけで足ります。ひな形での SDK と MOD のビルドにネイティブツールは要りません。

これ以外は何も入れません。`wasm-tools`、`cargo-component`、`wasm-opt` は使いません。

## 3. プロジェクトの構成

動く最小構成がひな形です。

```text
my-mod/
  yak.toml              パッケージのマニフェスト
  src/lib.rs            MOD 本体（1 ファイルで足りる。no_std のクレートルート）
  build.sh / build.ps1  SDK のコンパイル、MOD のコンパイル、.yak のパック
  bin/wasm/<name>.wasm  ビルド出力（パスは yak.toml に書く）
  dist/<id>-<version>.yak  できあがったパッケージ
```

MOD は `#![no_std]` の cdylib のクレートルートで、SDK の `rlib` に対してコンパイルします。

```rust
#![no_std]
extern crate alloc;

use yoake_guest_sdk::{Handle, Registrar, export_guest, hooks, original};

fn init(r: &mut Registrar) { r.hook(hooks::SERVER_TICK, 0).expect("register SERVER_TICK"); }
fn hook(id: u32, root: Handle) -> i64 { let _ = original(root); 0 }

export_guest!(init, hook);
```

`export_guest!(init, hook)` が 2 つのエクスポート（`yoake_init`、`yoake_hook`）と ABI のカスタムセクション（第 13 章）を出力します。アロケータ（`alloc` が使えます）と、トラップするパニックハンドラは SDK が用意します。`std` はありません。

Cargo のプロジェクトでも、同じフラグ（`crate-type = ["cdylib"]`、`panic = "abort"`、第 8 章のリンカ引数）を指定して `crates/yoake-guest-sdk` をパス依存にすれば動くはずです。ただしこのツリーに Cargo で MOD をビルドする例はないので、ひな形では採用していません。

## 4. `yoake_init` とフックを書く

### 2 つの関数

`export_guest!(init, hook)` には 2 つの関数を渡します。

| 関数 | 呼ばれるとき | 役割 |
|---|---|---|
| `init(r: &mut Registrar)` | MOD の読み込み時に 1 回（`yoake_init`） | `r.hook(id, priority)` で欲しいフック ID を登録する |
| `hook(id: u32, handle: Handle) -> i64` | 登録したフックが発火するたび（`yoake_hook`） | 実際の処理。`match id` で処理を選ぶ。戻り値がフック対象の関数の結果になる |

フック ID のうち小さい数字は、エンジンの関数や呼び出し箇所の **台帳 ID** です。`0x8000_0000` 以上はエンジンが用意する合成フックで、このツリーでは `hooks::SERVER_TICK`（`0x8000_0001`）がそれです。台帳は `hook-ledger/`（`horizon_client.json`、`horizon_client_lib.json`）にあり、各エントリが `id`、`key`（関数の Rust パス）、`status`（`live` か `tombstone`）、`kind`（`fn` か `callsite`）を持ちます。登録してよいのは `live` の ID だけです。エンジンが実際に入れる ID は生成されたブリッジ（`crates/horizon-client/src/yoake_glue/hook_bridge_*.rs` の `BRIDGED`、`OBSERVE_ONLY`、`UNBRIDGED`）で決まります。このコミットでブリッジされているのは 55 個（関数フック 29、呼び出し箇所フック 26）です。読み込み時に `hook_register` が結果を返します（次のモードの表を参照）。

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

### `OUT_TOO_SMALL` とチケット

普段は見ることがありません。SDK の `call` が処理するからです。ただ、理由は知っておく価値があります。

- 生のインポートは `engine_call(handle, id, ptr, len, out_ptr, out_cap) -> i32` です。0 以上の戻り値は、書き込んだ結果のバイト数です。
- 結果が `out_cap` に **収まらなかった** とき、呼び出しは **すでに実行済み** で、結果は **チケット** に保管されています。戻り値は `-(チケット + 256)` 以下で、必要なサイズは `out_ptr` にリトルエンディアンの `u32` で書かれます。結果は `engine_take(ticket, out_ptr, out_cap)` で取り出します。
- **やり直しのために同じ呼び出しをもう一度しないでください。** セッター、スポーン、カウンターが 2 回実行されてしまいます。
- SDK が代わりにやってくれます。`call` はまず 256 バイトのバッファで試し、`OUT_TOO_SMALL` になったら必要な大きさだけ確保してチケットを取り出します。チケットが有効なのは、その activation の中だけです。1 activation が保持できる結果は最大 8 個、合計 16 MiB までで、それを超えるとステータス `-8`（`Error::Status(-8)`）になります。**呼び出しは実行されたのに結果は失われます。** 大きな結果をいくつも同時に保持する手書きのゲストでないと、ここには届きません。

## 6. クライアント側とサーバー側のモジュール

**このツリーでは**、フルアクセスのパッケージが宣言できるモジュールは **1 つ**、`[wasm] server` だけです。ワールドが始まるときにシミュレーションのスレッド（統合サーバーまたは専用サーバー）で動くコアモジュールです。`world = "full-access"` で `wasm.client` を宣言したり、`[side] server` を `true` にしなかったりすると、マニフェストは拒否されます。合成フックは `hooks::SERVER_TICK` だけで、引数 0 はサーバーの `Game` ルートです。

別ブランチ `yoake2/client-hooks`（コミット `c681140e0`、レポート `docs/yoake2/reports/MX3CL1.md`）は、**このツリーにはマージされていません**。そのブランチの文書によると、次のように変わります。

| 宣言 | 動く場所 |
|---|---|
| `[wasm] server = "bin/wasm/x.wasm"` | ワールド開始時のシミュレーションスレッド |
| `[wasm] client = "bin/wasm/y.wasm"` | クライアントセッションの準備ができたときの、ウィンドウのスレッド |
| 両方 | 両方のスレッドで、別々の 2 つのインスタンスとして（メモリもグローバルも共有しない） |

`[side]` は宣言したモジュールと完全に一致させる必要があります（`side.server` は `wasm.server` があるときだけ、`side.client` は `wasm.client` があるときだけ）。クライアント側も同じ許可リストと同じルールに従います。追加されるフックは `hooks::CLIENT_TICK`（`0x8000_0002`。クライアントの tick ごとに 1 回で、引数 0 はクライアントの `Game`）と `hooks::HUD_RENDER`（`0x8000_0003`。HUD のあと、フレームごとに 1 回。観察専用で、引数 0 は無効なハンドルなので、まだ HUD に描画はできません）です。これは先行プレビューとして読んでください。ブランチがマージされるまでは、`wasm.client` のあるパッケージは読み込まれず、SDK に `CLIENT_TICK` の定数もありません。

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

`rustc` を 2 回呼びます。`crates/yoake-guest-sdk/tests/support/build.rs`（エンジン自身のテストが使っているもの）と同じです。

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
interface = "1.0"
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
| `[wasm] interface` | `"1.0"`（メジャーは 1。マイナーが 1 以上だと別の world が必要になるので `1.0` を使う） |
| `[wasm] server` | `bin/wasm/` の **直下** にある、`.wasm` で終わるファイル。このツリーでは必須。`wasm.client` は不可（第 6 章） |
| `[wasm] permissions` | 指定不可。フルアクセスは全か無かだから |
| `[side]` | `server = true` |

ほかに `name_ja`、`license`、`homepage`、`dependencies`、`optional`、`conflicts` が使えます。`yak.toml` と同じ場所にある `README.md`、`LICENSE`、`content/`、`assets/`、`shaders/`、`config/` も一緒にパックされます。

### パックする

ソース一式にパックツール `cargo-horizon-mod` があります。ソース一式のルートで次を実行します。

```sh
cargo run -p cargo-horizon-mod -- pack path/to/my-mod
```

`yak.toml` を読み、宣言されたファイルを集めて、**署名なし** の `<id>-<version>.yak` をプロジェクトのフォルダに書き出します。同名のファイルがあると上書きしません（ひな形のスクリプトは古いファイルを消し、新しいファイルを `dist/` に移します）。出力の要約行には「data-only package」と出ますが、これはネイティブバイナリ（`[bin]`）の有無を指す言い回しで、WebAssembly モジュール入りでもこう表示されます。できあがりの確認は次のとおりです。

```sh
cargo run -p cargo-horizon-mod -- yak verify path/to/my_full_access_mod-0.1.0.yak
```

フルアクセス MOD に `yoake build`／`pack`／`check`（`crates/yoake-cli` の `yoake` CLI）は使わないでください。サンドボックス型の WebAssembly 向けに Cargo でコンポーネントをビルドし、サーバーモジュールもサンドボックス型のコンポーネントとして検査するツールだからです。

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

フルアクセスのゲストはすべて、`yoake_abi` という名前のカスタムセクションに ABI を宣言します。ちょうど 4 バイトで、メジャーバージョンをリトルエンディアンの `u16`、続けてマイナーバージョンをリトルエンディアンの `u16` で置きます。SDK とエンジンは **1.0**（`01 00 00 00`）で、`export_guest!` が出力します（SDK の `ABI_MAJOR`、`ABI_MINOR`）。ローダーはこのセクションを、**ゲストのコードを一切実行する前に** モジュールのバイト列から読みます。

| ゲストのセクション | 結果 |
|---|---|
| 無い、長さが違う、重複、壊れている | 拒否 |
| メジャーバージョンが違う | 非互換として拒否 |
| メジャーもマイナーも同じ | 読み込む |
| メジャーが同じでゲストのマイナーが新しい | 読み込むが、MOD がより新しいエンジンを想定している旨をログに出す。エンジンが「未知のバインディング」として断る呼び出しは失敗しうる |
| メジャーが同じでゲストのマイナーが古い | 読み込む（マイナーでの追加は後方互換） |

インポート、バインディング、ステータスの追加はマイナーの変更です。既存のインポートやバインディングの変更や削除、ステータスの意味の変更、ワイヤ形式の変更はメジャーの変更です。バインディング ID は追記のみで再利用されないので、ビルド済みの MOD は同じメジャーの範囲のエンジン更新をまたいで動き続けます。

MOD 自身の `yak.toml` の `version` は別物です。リリースのたびに上げてください。そのたびにプレイヤーは新しいファイルを許可することになります。

## 要確認（確かめられていないこと）

1. **クライアント側モジュールと `CLIENT_TICK`／`HUD_RENDER`** は、未マージのブランチ `yoake2/client-hooks`（`c681140e0`）にしかありません。第 6 章はそのブランチの `ENGINE_CALL_ABI.md` と `MX3CL1.md` の写しで、ここではビルドも実行もしていません。このツリーでは `wasm.client` を持つフルアクセスのマニフェストは拒否されます。
2. **プレイヤーが `yoake:` の行をどこで見られるか。** stderr と `log` クレートに出力されますが、ランチャーがゲームの stderr をどこに保存するかは見つけられなかったため、どのファイルを開くかは書いていません。
3. **実際のゲームでの動作。** ゲームは一度も起動していません。ひな形とサンプルは、本物のローダーと仮の `Game` ルート（パス `Game.experience` に本物の `Experience` を置いたもの）で動かしました。本物の `Game` に対して同じ SDK サンプルを動かすテストは `crates/horizon-client/src/game/yoake2_examples_tests.rs` にありますが、`HORIZON_TEST_RESOURCES` がないと無視されます。
4. **「許可したあとにゲームを起動（再起動）する」** はコードからの推論です（ランチャーはゲームを起動する前にリストを書き、ゲームは最初のワールド開始時に 1 度だけ読む）。動作中のランチャーでは確認していません。
5. **呼び出し箇所フック**（台帳の `kind = callsite`）: 台帳とブリッジでは関数フックと同じように並んでいて、`docs/yoake2/HOOK-SLICE-BRIDGE.md` には、ゲストから見えるのは引数 0 のハンドルで、ほかの引数は見えないとあります。このガイドのために呼び出し箇所フックを動かしてはいません。
6. **フルアクセス MOD に `yoake` CLI を使わないこと**: `crates/yoake-cli/src/main.rs` を読んでの判断です（Cargo でコンポーネントをビルドし、サーバーモジュールをサンドボックス型のコンポーネントとして検査する）。フルアクセスのパッケージに対して実行はしていません。
7. **`build.sh` は Windows の Git Bash でしか実行していません。** Linux と macOS は試していません。この Windows（GNU ツールチェーン）のマシンでは、パックツールのコンパイルに MinGW（`dlltool`、`as`）のフォルダを `PATH` に通す必要がありました。
8. **フック ID は安定した公開一覧ではありません。** 台帳には `tombstone` のエントリやブリッジされていない ID があります。ブリッジ済みのフックが 55 個というのはこのコミット時点の数です。
9. **フック 1 回あたり 110〜150 ns** は S2 スパイクのレポート（`MX3S2.md`）にある、1 台のマシンでの測定値です。桁の目安として読んでください。
