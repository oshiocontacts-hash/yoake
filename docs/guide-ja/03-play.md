# 3. ゲームに入れて遊ぶ・すぐ試す

MOD をビルドしたら、実際に Yoake のゲーム内で動かしてみましょう。
**WIT ワールドが `client-mod` の MOD を開発フォルダですぐ試す方法**と、**すべての MOD を `.yak` パッケージとして入れる方法**があります。

---

## 3-1. `client-mod` を開発フォルダですぐ試す

WIT ワールドが `client-mod` の MOD なら、毎回 `.yak` を導入し直す代わりに、生の `.wasm` を開発フォルダから読み込んでホットリロードできます。開発フォルダはマニフェストを読まず、モジュールは常に `client-mod` として扱われます。[^dev-dir]

> **注意**: 今の版では `YOAKE_DLL_MODS_UNSIGNED=1` は WebAssembly 専用のスイッチではありません。ゲームのプロセス全体に適用され、**PC 上のあらゆる操作が可能な、署名のないネイティブ DLL MOD も許可します**。WebAssembly だけを許可するスイッチは、エンジン側で追加が予定されています。[^unsigned]
>
> - **常駐させないでください**。`setx` は絶対に使わず、Windows やユーザーの環境変数にも登録しないでください。
> - **推奨**: ランチャーの設定の "Developer options"（サイドバーの "Launcher x.y.z" を 7 回クリックすると出てくる）→「自作の DLL MOD を試す（開発者モード）」を使います。この設定はランチャーから起動したゲームだけに適用されます。[^launcher]
> - 環境変数を使う場合は、その 1 つの PowerShell ウィンドウだけで `$env:YOAKE_DLL_MODS_UNSIGNED='1'` と設定し、**同じウィンドウからゲームを直接起動**します。ランチャーの設定がオフなら、ランチャーはゲームの起動環境からこの変数を削除します。[^launcher]
> - 有効な間は**自分が作った `.yak` だけ**をインストールし、`YOAKE_DLL_MODS` は空に保ちます（PowerShell では `$env:YOAKE_DLL_MODS=''`）。終わったら PowerShell ウィンドウを閉じてください。

### 開発フォルダの設定と `yoake dev`

ランチャーを通さずゲームを直接起動する場合は、上の注意に従って PowerShell で `YOAKE_DLL_MODS_UNSIGNED` を設定します。続けて、同じウィンドウで開発フォルダを指定します。

```powershell
$env:YOAKE_WASM_DEV_DIR='<フォルダのパス>'
```

**同じ PowerShell ウィンドウからゲームを直接起動**すると、指定フォルダの `<id>.wasm` を `.yak` なしで読み込めます。ランチャー経由では、ランチャー側の設定がオフのとき `YOAKE_DLL_MODS_UNSIGNED` が削除されるため、この PowerShell 設定は使えません。MOD のプロジェクトフォルダでは、次のコマンドを実行します。

```sh
yoake dev
```

`yoake dev` はビルドと検査を行い、WIT ワールドが `client-mod` で `YOAKE_WASM_DEV_DIR` が設定されていれば、ビルドした `.wasm` をそのフォルダへコピーします。[^cli]

`client-full` / `server-full` の MOD ではコピーせず、`ok:true`、`installed:false` の結果に次の note を出力します。これはエラーではないため、次節の `.yak` の手順を使ってください。

```text
raw Wasm hot reload only supports client-mod; not installed
```

### ホットリロード（ゲームを起動したまま更新）

ゲームは 1 秒おきに、起動時に見つけた `.wasm` の更新を確認します。

- **ファイルの書き込みが終わって 1 秒間変化がなければ**、次のフレームまたはティックで新しいインスタンスに自動で差し替わります。[^watch]
- 新しい `.wasm` の読み込みに失敗した場合は、1 つ前の正常なインスタンスが動き続け、ログに警告が出ます。
- **開発フォルダ内のファイルはゲーム起動時にだけ発見されます**。起動後に新しいファイルを置いた場合は、ゲームを再起動してください。起動時にあったファイルだけが、その後のホットリロード対象になります。
- ホットリロードは WIT ワールドが `client-mod` のモジュールだけです。`client-full` / `server-full` は対象外です。

---

## 3-2. `.yak` パッケージをインストールする

`yoake pack` を実行すると、プロジェクト内の `dist/<id>-<version>.yak` にパッケージが出力されます。

```sh
yoake pack
```

### 新しいサンプルとフルカタログ MOD の手順

`hud-item`、`server-storage`、`config-demo`、`permission-demo`、`mod-call-pair`（`caller` と `responder`）、`chest-count`、`item-trade`、`player-vitals` はすべてフルカタログの MOD なので、開発フォルダではなく、必ず次の手順でインストールします。

1. プロジェクトで `yoake pack` を実行します。
2. 上の安全上の注意を確認し、ランチャーの設定の "Developer options"（サイドバーの "Launcher x.y.z" を 7 回クリックすると出てくる）→「自作の DLL MOD を試す（開発者モード）」を有効にします。
3. ランチャーの MOD 画面から、ローカルの `.yak` ファイルを選んでインポートします。[^local-yak]
4. ゲームを再起動します。

### ゲーム本体の読み込みの仕組み

パッケージの展開と一覧ファイルの管理は次のように行われます。[^package]

- `.yak` の展開（`<game dir>/yoake-mods/<id>/<version>/` への配置と `yoake-mods/.cache/` へのアーカイブ保持）と `<game dir>/yoake-mods.txt`（1 行につき `<id> <version>`）の更新は**ランチャー**が行います。ゲーム本体は展開済みのフォルダを読み込みます。
- 専用サーバー（`--dedicated`）でも、`--game-dir` の `yoake-mods.txt` に書かれたパッケージからサーバーモジュール（WIT ワールド `server-full`）を読み込みます。
- 未署名の手作り `.yak` には開発者モードが必要です。有効な間は、インストール済み `.yak` 内の未署名 `client-mod` モジュールもホットリロード監視の対象になります。

---

## 3-3. 動作確認とログの見方

ゲームが起動すると、WebAssembly エンジンが動いているかどうかが起動ログに 1 行出力されます。

- MOD が 1 つも無いとき: `wasm: no mods (engine not started)`
- MOD が読み込まれたとき: `wasm: N mod(s), engine started`（`N` は読み込まれた MOD の数）

MOD の中から出力したログ（`yoake_wasm_sdk::log(...)` や、標準ライブラリ有効時の `println!` / `eprintln!`）もゲームのログに出ます。HUD が出ないときやコマンドが動かないときは、まず起動ログを確認しましょう。

---

## 次のステップ

MOD の動かし方が分かったら、次は [4. サンプル MOD 集を読む（`04-examples.md`）](./04-examples.md) に進んで、アイテムアイコンの描画・コマンド追加・データ保存・設定画面・権限チェックの具体的な書き方を見ていきましょう。

[^dev-dir]: 確認元: `crates/horizon-client/src/wasm_mods.rs` の `dev_sources`。
[^unsigned]: 確認元: `crates/horizon-client/src/dll_mods.rs` と `docs/api/yoake-mods.md` の署名検証・ネイティブコードの説明。
[^launcher]: 確認元: `launcher/src/dev_options.rs` と `launcher/src/backend.rs` の `set_unsigned_dll_env` / `client_command_for_program`。
[^cli]: 確認元: `.lane/ref/p10-tools-yoake-cli-main.rs` の `dev_project` / `install_for_reload`。
[^watch]: 確認元: `crates/horizon-client/src/wasm_mods.rs` の `Watch::poll` / `spawn_watcher`。
[^local-yak]: 確認元: `launcher/src/yoake_mods_page.rs` の `add_local_yak` / `choose_local_yak`。
[^package]: 確認元: `docs/api/yoake-mods.md` と `docs/yoake-wasm-dev.md`。
