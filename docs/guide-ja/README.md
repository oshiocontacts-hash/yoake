# Yoake WebAssembly MOD 制作入門ガイド

このガイドは、**Yoake** 向けの MOD をはじめて作る方のための日本語入門書です。
プログラミングに少し触れたことがあれば、**Rust** や **WebAssembly（Wasm）** を使うのが初めてでも順番に読み進められるように書いています。

---

## WebAssembly MOD とは？

Yoake の MOD は、Rust で書いたプログラムを **`wasm32-wasip2` という種類の WebAssembly** にビルドし、設定ファイル（`yak.toml`）と一緒に `.yak` パッケージにまとめて配布・導入します。

WebAssembly MOD はゲーム本体の中の安全なサンドボックス（隔離環境）で動きます。万が一 MOD の中で無限ループや時間切れが起きても、その MOD だけが止まる仕組みになっています。

> [!NOTE]
> このガイドは `.yak`（Wasm）で MOD を作る方法を説明します。DLL のネイティブ MOD も使えますが、持ち主や開発者向けの上級の道で、署名のない DLL は開発者モードでしか動きません。このガイドでは扱いません（署名の考え方だけ [7. MOD を公開する](./07-publishing.md) で触れます）。

---

## この入門で作れるもの

このガイドと付属のサンプルを通じて、次のような MOD を作れるようになります。

1. **画面（HUD）のカスタマイズ**
   - ゲーム画面の好きな位置に、半透明のパネル・文字・アイテムアイコン（ダイヤモンドなど）を描画する
   - 標準のホットバーなど画面の部品を隠す、ツールチップに説明文を足す、視野角（FOV）を変える
2. **オリジナルのスラッシュコマンドとデータ保存**
   - `/visits` のような独自コマンドを追加し、ワールドごと・プレイヤーごとの記録を保存してチャットで返事をする
3. **ゲーム内設定画面（Configure）との連動**
   - `yak.toml` の `[config]` に設定項目（文字列や選択肢など）を定義し、MOD 一覧の「Configure」ボタンから変更した内容を即座に MOD に反映する
4. **権限ノード（`[permissions."<node>"]`）のチェック**
   - `yak.toml` の `[permissions."<node>"]` で権限ノードを定義し、OP レベルやワールドごとの `permissions.toml` に応じて `/secret` などのコマンド実行を許可・拒否する
5. **テスト・公開・エンジン関数への割り込み**
   - ゲームエンジンの中で自動テストを実行し、署名とランチャーのカタログの仕組みを学び、Mixin でエンジンの関数に割り込む

---

## 目次と読む順番

上から順番に読むことで、環境づくりから MOD の作成・テスト・公開・応用まで一通り体験できます。

### 1. [環境準備（`01-setup.md`）](./01-setup.md)
- Rust ツールチェーン（`rustup`）のインストール
- WebAssembly ビルド用ターゲット（`rustup target add wasm32-wasip2`）の追加
- `yoake` CLI の入れ方（`cargo install --path crates/yoake-cli`）
- エディタと作業フォルダの準備（Windows を中心に、macOS / Linux も補足）

### 2. [最初の MOD を作る（`02-first-mod.md`）](./02-first-mod.md)
- `yoake new hud-only my-hud` でひな形を作る（5 種類のテンプレート一覧つき）
- 作られるファイル（`yak.toml` / `Cargo.toml` / `src/lib.rs` / `README.md` / `.yoake/` など）の役割
- `yoake build` → `yoake check` → `yoake pack` の流れと、出力される `dist/<id>-<version>.yak` の確認

### 3. [ゲームに入れて遊ぶ・すぐ試す（`03-play.md`）](./03-play.md)
- パッケージ（`.yak`）のインポートとゲームの読み込みの仕組み
- WIT ワールドが `client-mod` の MOD を `YOAKE_WASM_DEV_DIR` と `yoake dev` でホットリロードする方法と、Wasm の開発者モードの安全上の注意

### 4. [サンプル MOD 集を読む（`04-examples.md`）](./04-examples.md)
- リポジトリ内の 19 個のサンプル（`hello`、`hello-std`、`hud-item`、`server-storage`、`config-demo`、`permission-demo`、`full-demo`、`full-services`、`mod-call-pair`、`chest-count`、`item-trade`、`player-vitals`、`content-demo`、`mixin-demo`、`test-demo`、`tested-counter`、`client-services-demo`、`gpu-demo`、`ime`）の解説
- 各サンプルが使うフック（`hook`）とホストサービス（`services`）の一覧
- `yak.toml` での `[config]`、`[wasm] permissions`、`[permissions."<node>"]` の書き方

### 5. [よくあるエラーと直し方（`05-errors.md`）](./05-errors.md)
- フック名の書き間違いや、クライアント／サーバーのフック取り違え
- `.wasm` のビルドし忘れ（stale エラー）や Windows の予約語エラー
- `YOAKE_WASM_DEV_DIR` 未設定や `wasm32-wasip2` 未追加のエラー
- `yak.toml` の代表的なバリデーションエラーと修正方法

### 6. [MOD をゲームエンジンの中でテストする（`06-testing.md`）](./06-testing.md)
- `yoake test` と `horizon-client --yoake-test` によるゲームエンジン内の自動テスト
- `TestWorld` と `TestDriver` の全関数リファレンス、スーパーフラットワールドの前提
- 2 クライアントのマルチプレイテスト（`--clients 2` と `ClientView`）
- よくある失敗と未実装の機能

### 7. [MOD を公開する（`07-publishing.md`）](./07-publishing.md)
- `yak.toml` のバージョンの付け方と、`.yak` パッケージの中身・サイズ上限
- 署名の仕組みと、署名のない MOD を開発で動かすための 2 つのスイッチ
- ランチャーの MOD カタログ（`index.json`）への載せ方、更新の出し方、公開時にやってはいけないこと

### 8. [Yoake Mixin 入門（`08-mixin.md`）](./08-mixin.md)
- エンジン関数の決められた場所へ割り込む仕組み（HEAD・RETURN・overwrite）
- `inject-mod` の `yak.toml` の書き方、値の範囲と拒否、コマンド・チャットへの割り込み
- 順番と競合、上限とトラップ、よくある失敗

### 9. [アイテム・ブロックスプライトの動的差し替え（`09-sprites.md`）](./09-sprites.md)
- `item-sprite-set` / `item-sprite-reset` による共有アトラス矩形の書き換え
- `block-sprite-set` / `block-sprite-reset` によるブロック層とミップマップの再生成
- サイズ不一致・他 MOD 上書き・リソースパック再読み込み・アンロード時の挙動

### 10. [MOD 画面で IME（日本語入力）を使う（`10-ime.md`）](./10-ime.md)
- `IME_PREEDIT` / `IME_COMMIT` の購読と、変換途中文字列の描画
- OS の IME 有効化と、候補ウィンドウの GUI スケーリング済み座標での配置
- 既存の文字フックとの二重入力回避、MOD 画面の所有権とライフサイクル

### 11. [画面・インベントリ・音・食料のサービス（`11-small-services.md`）](./11-small-services.md)
- 画面の入力フックからの送信、回転したテクスチャー、インベントリの読み取り
- クライアントだけの音、食料の設定、ブロックエンティティの書き戻し、キーの消費

---

## おすすめの進め方

- **まずは動かしてみたい方**: [`01-setup.md`](./01-setup.md) → [`02-first-mod.md`](./02-first-mod.md) → [`03-play.md`](./03-play.md) の順に進めて自分の画面に文字を出したあと、[`06-testing.md`](./06-testing.md) でゲームエンジン内の自動テストを試してみましょう。
- **作りたい機能が決まっている方**: 一通り動かしたあと、[`04-examples.md`](./04-examples.md) から近いサンプルを探してコードを真似し、[`06-testing.md`](./06-testing.md) でサーバー側の動作を自動検証するのが近道です。
- **作った MOD を他の人に配りたい方**: [`07-publishing.md`](./07-publishing.md) で、バージョンの付け方・署名・更新の出し方を確認しましょう。
- **ゲームの処理そのものに割り込みたい方**: [`08-mixin.md`](./08-mixin.md) で Mixin（`inject-mod`）の書き方を読みましょう。
- **エラーが出たとき**: コマンドが出力するメッセージやゲームのログを見て、[`05-errors.md`](./05-errors.md)（テスト実行時のエラーは [`06-testing.md`](./06-testing.md) の 6-6 節）を辞書代わりに引いてください。
