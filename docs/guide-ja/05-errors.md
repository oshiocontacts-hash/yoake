# 5. よくあるエラーと直し方

MOD を作っているときに出会いやすいエラーメッセージと、その原因・直し方をまとめました。
`yoake` コマンドが出力する JSON の `"errors"` 欄や、ビルド時のメッセージ、ゲームのログと照らし合わせて確認してください。

---

## 5-1. フック（`subscriptions`）に関するエラー

### ① `unknown hook subscription: <name>`

- **原因**: `subscriptions()` が返したフック名 `<name>` が存在しない名前（タイプミスなど）になっています。または、WIT ワールドが `client-mod` の標準モジュールでは使えないフック名を書いた場合にも発生します。
- **直し方**:
  - フック名を直接文字列（`"hudd"` など）で手書きせず、SDK が用意している定数 `hook::HUD` や `yoake_wasm_sdk::full::hook::*` を使ってください。
  - フルカタログ（client-full / server-full）のフックを使う場合は、`yak.toml` の `[wasm]` に `world = "client-full"` を指定し、`ClientMod` / `export_client_full!` を使っているか確認してください。

```rust
    fn subscriptions() -> &'static [&'static str] {
        &[hook::HUD]
    }
```

### ② `hook hud runs on the client side and cannot be subscribed from a server module`

- **原因**: クライアント専用フック（この例では `hud`）を、サーバーモジュール（WIT ワールド `server-full`）の `subscriptions()` で購読してしまっています（逆に、サーバー専用のフックをクライアントモジュールで購読した場合は `hook <name> runs on the server side and cannot be subscribed from a client module` になります）。
- **直し方**:
  - 画面描画（`hook::HUD` など）はクライアントモジュール（`ClientMod`）側で購読してください。
  - サーバーモジュール（`ServerMod`）側では、`hook::REGISTER_COMMANDS` や `hook::PERFORM_COMMAND`、`hook::SERVER_TICK` などサーバー側または共通（both）のフックだけを購読してください。

---

## 5-2. `yoake` コマンド・開発ツールに関するエラー

### ③ `... is stale because ... is newer; run yoake build`

- **原因**: `src/lib.rs` や `Cargo.toml` などのソースファイルを編集したあと、まだビルドしていないため、`bin/wasm/*.wasm` のファイルがソースコードより古い（stale）状態のまま `yoake check` を実行しました。
- **直し方**: `yoake check` は検査だけを行いビルドはしません。次のコマンドを実行して `.wasm` をビルドし直してください。

```sh
yoake build
```

### ④ `name "con" is a reserved Windows name`

- **原因**: `yoake new` で指定した名前（または `id`）に、Windows がシステム予約語として使っている名前（`con`、`prn`、`aux`、`nul`、`com1`〜`com9`、`lpt1`〜`lpt9`）を指定しました。
- **直し方**: 予約語を避けて、`my-hud` のような別の名前で `yoake new` を実行してください。

```sh
yoake new hud-only my-hud
```

### ⑤ `YOAKE_WASM_DEV_DIR is unset; not installed`

- **これはエラーではありません**。ビルドと検査は完了しており、結果は `ok:true`、`installed:false` です。開発フォルダが未設定なのでコピーしなかったという note です。[^dev-cli]
- WIT ワールドが `client-mod` の MOD を開発フォルダにコピーしたい場合は、[3 章の安全上の注意と設定方法](./03-play.md)を確認し、その PowerShell ウィンドウだけで `$env:YOAKE_WASM_DEV_DIR='<フォルダのパス>'` を設定してから `yoake dev` を実行します。開発フォルダの `.wasm` をゲームが読むには `YOAKE_WASM_UNSIGNED=1`（またはランチャーの「自分の WASM MOD をテスト」）も必要で、無いと黙って読み込まれません。
- `client-full` / `server-full` の MOD には、代わりに `raw Wasm hot reload only supports client-mod; not installed` という note が出ます。これも `ok:true`、`installed:false` の結果で、開発フォルダにはインストールされません。`yoake pack` で `.yak` を作り、ランチャーからインポートしてゲームを再起動してください。

### ⑥ `wasm32-wasip2` ターゲットが無いときの Cargo のエラー

- **症状**: ビルド時に Cargo / rustc から `wasm32-wasip2` ターゲットが見つからない（`can't find crate for 'core'` / `target may not be installed` など）というエラーが出る。
- **原因**: Rust に WebAssembly コンポーネント用のビルドターゲット `wasm32-wasip2` がまだ追加されていません。
- **直し方**: ターミナルで次のコマンドを実行してターゲットを追加してください。

```sh
rustup target add wasm32-wasip2
```

---

## 5-3. `yak.toml` の書き間違い（代表的なエラー）[^validation]

`yak.toml` に記入ミスがあると、`yoake check` や `yoake build`、ゲーム起動時の読み込みで次のようなエラーが出ます。

### ⑦ `invalid YAK identifier: <id>`

- **原因**: `id`（または依存関係やシェーダー／サービスの ID）の文字数や文字種がルールに合っていません。
- **直し方**: `id` は **3 文字以上 48 文字以下** で、使える文字は **英小文字（`a-z`）・数字（`0-9`）・アンダースコア（`_`）・ハイフン（`-`）のみ** です。大文字やスペース、2 文字以下の名前は使えません。ただし `yoake new <template> <name>` の `<name>` では `_` を使えません（[2 章の 2-1](./02-first-mod.md)）。

### ⑧ `wasm.interface must be major.minor, for example 1.0`
（または `unsupported wasm interface major version ...` / `wasm interface ... is newer than the supported 1.1` / `wasm interface 1.1 requires wasm.world = "inject-mod"; ordinary hook worlds use 1.0`）

- **原因**: `[wasm]` の `interface` が `"1.0"` のような `メジャー.マイナー` 形式になっていないか、現在のビルドが対応しているバージョン以外になっています。対応しているのは `"1.0"` と、`world = "inject-mod"` のときだけ使える `"1.1"` です。
- **直し方**: 通常の MOD の `[wasm]` セクションには `interface = "1.0"` と書いてください。Mixin（`inject-mod`）だけは `interface = "1.1"` です（[8 章](./08-mixin.md)）。

### ⑨ `wasm.client must be a file directly under bin/wasm/ ending in .wasm`
（サーバー側の場合は `wasm.server must be a file directly under bin/wasm/ ending in .wasm`）

- **原因**: `[wasm]` の `client` や `server` に指定したパスが、`bin/wasm/<ファイル名>.wasm` の形式になっていません（サブフォルダを入れていたり、`target/...` を直接指定していたり、拡張子が `.wasm` でない場合など）。
- **直し方**: `"bin/wasm/hud-item.wasm"` のように、`bin/wasm/` 直下の `.wasm` ファイル名を指定してください。

### ⑩ `unknown wasm.world "..." (known: client-mod, client-full, inject-mod)`

- **原因**: `[wasm]` の `world` に `"client-mod"`・`"client-full"`・`"inject-mod"` 以外の文字（たとえば `"server-full"` など）を書いています。
- **直し方**: `world` には `"client-mod"`（省略時のデフォルト）、`"client-full"`、`"inject-mod"` のいずれかを指定してください。サーバーモジュールは `server = "bin/wasm/..."` を書くだけで、WIT ワールドが自動的に `server-full` になります（`inject-mod` の場合は [8 章](./08-mixin.md) を参照）。

### ⑪ `wasm.world applies to the client module; declare wasm.client`

- **原因**: サーバー専用の MOD（`wasm.client` が無い状態）なのに、`[wasm]` に `world = "..."` を書いてしまっています。`inject-mod` だけは `wasm.client` が無くても `world` を書けます。
- **直し方**: サーバー専用 MOD の場合は `[wasm]` から `world` の行を削除し、次の形にします。

```toml
[wasm]
interface = "1.0"
server = "bin/wasm/server-storage.wasm"
permissions = ["service.storage"]
```

### ⑫ `unknown wasm permission "..." (known: service.chat, service.commands, service.config, service.http, service.permissions, service.storage)`

- **原因**: `[wasm]` の `permissions` リストに、存在しないサービス権限名を書いています。
- **直し方**: 指定できるのは `"service.chat"`, `"service.commands"`, `"service.config"`, `"service.http"`, `"service.permissions"`, `"service.storage"` の 6 種類のみです。スペルを確認してください。

### ⑬ `wasm permission service.commands applies to the server module; declare wasm.server`
（または `wasm permission service.permissions applies to the server module; declare wasm.server`）

- **原因**: `"service.commands"` や `"service.permissions"` はサーバーモジュール専用のサービス権限ですが、`[wasm]` に `server = "bin/wasm/..."` が宣言されていません。ただし `"service.commands"` は、`inject-mod` ではサーバーモジュールが無くてもかまいません。
- **直し方**: サーバーモジュール（`wasm.server`）を宣言するか、クライアント専用 MOD であれば不要なサービス権限を `permissions` から外してください。

### ⑭ `wasm permission service.chat requires wasm.client`

- **原因**: `"service.chat"` を宣言したのに、クライアントモジュール（`wasm.client`）がありません。`service.chat` は、クライアント側のチャット・コマンドの文字に割り込む Mixin のためのサービス権限です。
- **直し方**: クライアントモジュールを宣言するか、不要なら `permissions` から `"service.chat"` を外してください（[8 章](./08-mixin.md)）。

### ⑮ `[wasm] needs at least one of client or server` / `at least one side must be enabled`

- **原因**:
  - `[wasm] needs at least one of client or server`: `[wasm]` セクションを書いたのに、`client` も `server` も指定されていません。
  - `at least one side must be enabled`: `[side]` セクションで `client = false` かつ `server = false` になっています。
- **直し方**: MOD が動く側（クライアントまたはサーバー、あるいは両方）を有効にし、対応する `.wasm` パスを指定してください。

### ⑯ `[config]`・`[permissions]`・`[http]` の宣言エラー

- `config <name>: value is not a declared enum choice`: `type = "enum"` の設定項目で、`default` の値が `values = [...]` の選択肢の中に含まれていません。
- `config <name>: integer outside declared range`（または `float outside declared range`）: `default` の数値が `min`〜`max` の範囲外になっています。
- `invalid permission declaration: <name>`: `[permissions."<node>"]` の名前が不正か、`default_op_level` に `0`〜`4` 以外の数値を指定しています。
- `[http] requires wasm permission service.http` / `service.http requires [http] hosts`: `[http]` セクションと `"service.http"` 権限は、必ず一緒に宣言してください。
- `"permission denied: ..."`（実行時エラー）: コード内で `services::config_get`、`services::storage_get`、`services::has_permission` などを呼び出しましたが、`yak.toml` の `[wasm] permissions` に対応する権限（`"service.config"`、`"service.storage"`、`"service.permissions"`）が書かれていません。

---

## 次のステップ

エラーの直し方が分かったら、次は [6. MOD をゲームエンジンの中でテストする（`06-testing.md`）](./06-testing.md) に進んで、MOD をゲームエンジンの中で自動テストする方法を見ていきましょう。

[^dev-cli]: 確認元: `crates/yoake-cli/src/main.rs` の `dev_project`。
[^validation]: 確認元: `crates/horizon-mods/src/yak/manifest.rs` と `crates/horizon-mods/src/yak/config.rs`。
