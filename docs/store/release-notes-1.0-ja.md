# Yoake 1.0 リリースノート（案）

Yoake 1.0 は、Minecraft: Java Edition 26.3 と互換性を持つ Rust 製のゲームクライアントとランチャーのリリースです。

## 1. WebAssembly（Wasm）MOD（.yak パッケージ）

- Yoake の MOD は、Rust からビルドした Wasm コンポーネント（`wasm32-wasip2`）を `.yak` パッケージにまとめて導入するのが基本です。Java 用 MOD の JAR ファイルは動作しません。
- Wasm MOD はゲーム内の隔離環境で動作し、メモリ上限（64 MiB）、呼び出しごとおよび 50 ms 窓ごとの実行時間上限と CPU（fuel）上限、`yak.toml` で宣言したサービス権限によって制限されます。
- 基本の 19 フックを使う `client-mod` のほか、クライアント側のフルカタログを扱う `client-full` と、統合サーバーおよび専用サーバーで動く `server-full` に対応しています。
- 通常の環境でプレイヤーが Wasm MOD を動かすには、ピン留めされた Yoake の署名鍵による有効な署名（`SIGNATURE`）が必要です。MOD 作者がパッケージの署名やポータル（カタログ）への掲載・更新を申請する窓口と手順は（未確定）です。
- 選んだベースパックと MOD の組み合わせはランチャーで「構成」として保存でき、YK1 共有コードを使って他の人と共有したり新しいインスタンスとして導入したりできます。

## 2. SDK と yoake CLI

- Rust 製のゲスト SDK（`yoake-wasm-sdk`）により、画面（HUD）描画、独自コマンド、設定、スコープ別・キー単位のストレージ保存、サーバーサービス呼び出し、MOD 間呼び出し（`mod-call`）などを組み込めます。標準ライブラリ（`std`）が既定で使えるほか、`#![no_std]` にも対応しています。
- `yoake` CLI は公開リポジトリから `cargo install --path crates/yoake-cli` でインストールできます。GitHub Releases での実行ファイル配布は（未確定）です。
- `yoake new` でテンプレートからプロジェクトのひな形を作成し、`yoake build`（ビルドと検査）、`yoake check`（`yak.toml` と `.wasm` の検査）、`yoake pack`（`dist/<id>-<version>.yak` の作成）までを行えます。各コマンドは実行結果を 1 行の JSON で標準出力に返します。
- 開発者向け設定で自作 Wasm MOD のテストを有効にすると、`client-mod` のモジュールは `yoake dev` と開発フォルダ（またはインストール済みの未署名 `.yak`）を使って、ゲームを起動したまま 1 秒おきの更新確認でホットリロードできます。

## 3. テストランナー

- `#[yoake_test]` と `export_tests!` で登録したテスト関数を、配布用の `.yak` パッケージには含まれない別コンポーネントのテストモジュール（`bin/wasm/<id>-tests.wasm`）としてビルドできます。
- `yoake test`（または `horizon-client --yoake-test`）を実行すると、ウィンドウを表示しないオフスクリーン状態でテストごとに新しいスーパーフラットワールドが起動し、実際のゲームエンジン内でテストが自動実行されます。
- ワールドはウォームアップ後に凍結され、`TestWorld` のブロック設置・破壊、エンティティやマネキンの召喚、コマンド実行、`mod_call` と `run_ticks` を使って、指定したティック数だけ正確にサーバーを進めて結果を検証できます。ゲームを起動せず `cargo test` と手元のモック（`TestDriver`）だけで素早く試すことも可能です。
- `--clients 2`（`--yoake-test-clients=2`）を指定すると、2 つ目のクライアントをループバック接続で参加させ、ブロック変更やチャットの同期を確認するマルチプレイテストも行えます（クライアント専用の HUD 描画などの検証やオーバーワールド以外のディメンション操作は未実装です）。

## 4. Mixin

- Yoake Mixin（`interface = "1.1"`、WIT ワールド `inject-mod`）を使うと、エンジンがカタログに公開している **51 個**の割り込み口（サイト）に対して、Wasm コンポーネントから処理を挟み込めます。パッケージの署名・identity・fingerprint・ABI・エンジンバージョンの検査は Mixin でも省略されません。
- 各サイトで、関数本体の前に引数を確認・変更・キャンセルする **HEAD**、戻り値を置き換える **RETURN**、本体を実行せず結果を返す **overwrite** の 3 種類を使えます。同じサイト・同じサイドの overwrite は 1 つの MOD だけ登録でき、HEAD と RETURN は MOD ID 順に重ねられます。
- 置き換える数値やブロック状態 ID はサイトごとの型幅と許容範囲で検査され、範囲外の値や不正なスコープ付きハンドル操作は丸められずに拒否されます。コマンドやチャットのサイトには `service.commands`（クライアント側チャットにはさらに `service.chat`）の宣言が必要で、書き換えたコマンドにも通常のコマンドと同じ安全上の制限がかかります。
- トラップや不正な応答が起きてもゲーム全体は止まらず、そのサイト・サイド・割り込みポイントのハンドラーを無効化してネイティブ処理へフォールバックします。

## 5. GPU（予定）

- GPU に関する新たな拡張機能は今後の**予定**であり、Yoake 1.0 の時点では未実装（詳細未確定）です。
- 現在のゲームクライアントは、Windows の Vulkan または DirectX 12、Linux の Vulkan または OpenGL 3.3、および macOS（Apple Silicon、または次の Nightly からの Intel Mac）の GPU 環境で動作し、グラフィックスプリセットやシェーダーパックに対応しています。

## 6. フレンドとグループ

- シングルプレイのワールドをそのまま公開して招待コードを発行し、フレンドがゲーム内またはランチャーにコードを入力して参加できます。通信はパスワード認証鍵交換（SPAKE2）と ChaCha20-Poly1305 で暗号化され、共有を止めるかワールドを閉じるとその招待コードは無効になります。
- ルーターのポート開放（PCP / NAT-PMP / UPnP）や STUN による公開アドレス確認に対応し、直接接続できないネットワーク環境ではポート転送、Tailscale などの VPN、または自分で動かせる中継サーバー（`horizon-relay`）による短縮コード接続も使えます。
- ランチャーの「フレンド」ページでは、フレンドコード（`YKF-XXXX-XXXX`）によるリクエストの送受信、グループの作成と招待コード（`YKG-XXXX-XXXX`、使用回数・有効日数の指定や取り消しに対応）での参加、メンバーの役割変更やブロックを 4 つのタブ（フレンド・リクエスト・グループ・ブロック）で管理できます。
- ランチャーのフレンドとグループ機能には Yoake ポータルへのサインインが必要で、ポータル側で有効になるまでは「まだ使えません」と表示されます。ワールドの招待コードとは別の仕組みであり、フレンドページが使えない間も招待コードや LAN、オンラインサーバーでのマルチプレイには影響しません。

## 7. ランチャーの新しいガラス調デザイン

- ランチャーに新しいガラス調のデザインを採用し、ホーム画面やフレンド画面などで共通のタブ付きパネル、リスト行、入力欄、ポップオーバーメニューに統一しました。
- ダーク・ライト・システム設定に合わせた表示モードと、アクセントカラーの選択に対応しています。
- フレンドの削除やブロック、グループの削除・退出・メンバー追放、フレンドコードの再発行などの重要な操作には、誤操作を防ぐ 2 段階の確認ボタン（赤色の危険操作ボタンとキャンセルボタン）を備えています。

## 8. 日本語と英語

- ランチャーの表示言語は日本語（`ja`）と英語（`en`）の 2 言語に対応しています。
- 初回起動時は OS の優先言語が英語なら英語、それ以外の環境では日本語が自動で選ばれ、設定画面からいつでも切り替えられます。
- 日本語の翻訳テーブル（`launcher/lang/ja.json`）に未翻訳の文字列がある場合は英語の文言が表示され、設定ファイルに ja・en 以外の言語コードが入っていた場合は日本語で表示されます。

## 出典メモ

※ この節は公開前に削除する内部メモです。

- **リード文（互換バージョン）**: `README.md`、`docs/store/store-listing-ja.md`。
- **1. WebAssembly（Wasm）MOD（.yak パッケージ）**: `docs/store/store-listing-ja.md`、`.lane/ref/yoake-wasm-dev.md`（19 フック・64 MiB・50 ms 窓）、`.lane/ref/guides/guide-publish/docs/guide-ja/07-publishing.md`、`.lane/ref/guides/guide-en/docs/guide-ja/02-first-mod.md`、`.lane/ref/guides/guide-en/docs/guide-ja/03-play.md`。
- **2. SDK と yoake CLI**: `.lane/ref/yoake-wasm-dev.md`、`.lane/ref/guides/guide-en/docs/guide-ja/02-first-mod.md`、`.lane/ref/guides/guide-en/docs/guide-ja/03-play.md`、`.lane/ref/guides/sdk-examples-2/docs/guide-ja/03-play.md`。
- **3. テストランナー**: `.lane/ref/guides/guide-testing/docs/guide-ja/06-testing.md`、`.lane/ref/guides/guide-en/docs/guide-en/06-testing.md`。
- **4. Mixin**: `.lane/ref/guides/guide-mixin/docs/guide-ja/08-mixin.md`、`.lane/ref/guides/guide-mixin/docs/guide-en/08-mixin.md`（コマンドの関門は `yoake/mixin-m1` の `docs/yoake-mixin.md` と 877978546）。
- **5. GPU（予定）**: `README.md`、`docs/store/store-listing-ja.md`。
- **6. フレンドとグループ**: `docs/play-with-friends.md`、`launcher/src/friends_view.rs`、`launcher/src/friends_page.rs`（役割変更・招待の使用回数と有効日数）、`launcher/src/social.rs`、`docs/store/store-listing-ja.md`。
- **7. ランチャーの新しいガラス調デザイン**: `launcher/src/friends_view.rs`、`docs/store/store-listing-ja.md`。
- **8. 日本語と英語**: `launcher/src/i18n.rs`、`docs/store/store-listing-ja.md`。
