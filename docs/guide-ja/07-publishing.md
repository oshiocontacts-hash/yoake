# 7. MOD を公開する

自分の環境で動くWebAssembly MOD ができたら、次は他のプレイヤーに配布・公開する番です。この章では、`yak.toml` のバージョンの付け方、`.yak` パッケージの中身と制限、電子署名（SIGNATURE）の仕組み、ランチャーの MOD カタログ（目録、`index.json`）への載せ方、更新の出し方、そして公開時にやってはいけないことを順番に説明します。

---

## 7-1. バージョンの付け方（`yak.toml` の `version`）

### `version` の書式と検証ルール

`yak.toml` の `version` 欄は、パッケージライブラリ（`horizon_mods::yak`）によって **`semver::Version`（セマンティックバージョニング）** として解析されます[^yak-manifest]。`"1.0.0"` や `"0.3.1"` のように、`MAJOR.MINOR.PATCH` の3つの数字が揃った完全なバージョン番号（SemVer文字列）を書く必要があります[^yak-manifest][^api-yoake-mods]。

バージョン番号に関して、パッケージライブラリ・ランチャー・ゲームクライアントの各層で次の規則が検査されます。

- **カタログ（`index.json`）との一致（ライブラリ・ランチャーが検査）**: カタログの 1 つの MOD（パッケージ ID）には `1〜1,024` 個の重複しない完全なSemVer文字列を登録でき、各バージョンのエントリの `version` は `manifest.version`（`yak.toml` の `version`）と完全に一致していなければなりません[^api-yoake-mods][^yak-catalog]。
- **同梱コンテンツとの一致（ライブラリ・ゲームが検査）**: パッケージ内に `content/horizon-mod.toml` を含める場合、その内側の `id` と `version` も外側の `yak.toml` と同じでなければなりません[^api-yoake-mods][^yak-mods-client]。
- **マルチプレイでの MOD 同一性チェック（ゲームが検査）**: `[wasm]` パッケージは、パッケージの `id`、`version`、および `.wasm` ファイルのSHA-256ハッシュ（クライアントモジュールがあればそのSHA-256、サーバーのみの MOD ならサーバーモジュールのSHA-256）を合わせた 1 つの「同一性（identity）」を持ちます。`required_on_join = true` の MOD は、ワールドのホストと参加ゲストでこれらが一致しているかをゲームが確認します。ただし、ホットリロード中の（開発用の）モジュールは、バイト列が変わりうるので `required_on_join` のダイジェストを固定しません[^wasm-dev][^wasm-mods-client]。
- **同じバージョン番号の再利用禁止（ランチャーが検査）**: ランチャーで MOD を入れる際、すでに同じ `(id, version)` がインスタンスに入っているのにアーカイブの `sha256` が異なる場合は拒否されます（`"A different archive already uses this mod version"`）[^launcher-tx]。中身を少しでも変えたら、同じ `version` を使い回すことはできません。

### いつどの数字を上げるか

- ランチャーは、インストール済みのバージョンより大きいバージョンがカタログにあるとき（`version.version > installed.version`）にのみ更新（Update）として扱います[^launcher-plan][^launcher-rows]。そのため、新しいバージョンを出すときは必ず前より大きいSemVerにする必要があります。
- なお、MOD 作者がどの変更でメジャー・マイナー・パッチのどれを上げるべきかという運用上の基準は、参照元の仕様・コードには定められていません（未確定）。

---

## 7-2. `.yak` の中身と制限

### パッケージを作るコマンドと入るファイル

WebAssembly（`[wasm]`）MOD では、[2 章（`02-first-mod.md`）](./02-first-mod.md)・[3 章（`03-play.md`）](./03-play.md)で使った **`yoake pack`** コマンドを使います[^yoake-cli]。`yoake pack` は `[wasm]` のビルドと検査を行ったうえで `horizon_mods::yak::pack_directory` を呼び出し、プロジェクト内の **`dist/<id>-<version>.yak`** に未署名の `.yak` ファイルを出力します（実行するたびに上書きされます）[^yoake-cli][^yak-pack]。

```sh
yoake pack
```

一方、データ専用パッケージの作成、および `.yak` の検証には、`docs/api/yoake-mods.md` に記載されている `cargo horizon-mod pack` / `cargo horizon-mod yak verify` を使います（持ち主や開発者向けの上級の道であるネイティブ（`[bin]`）パッケージの作成にも使います。カレントまたは指定した MOD フォルダ直下に `<id>-<version>.yak` を出力します）[^api-yoake-mods]。

```text
cargo horizon-mod pack path/to/mod
cargo horizon-mod yak verify path/to/mod/example_program-1.0.0.yak
```

パック処理（`pack_directory`）は単一ディスクのZIP / ZIP64形式で、各エントリを無圧縮（`Stored`）として書き出します[^yak-pack]。なお、`.yak` の読み込み側（リーダー）は `Stored` と `Deflate` の両方を受け付けます[^api-yoake-mods][^yak-root]。アーカイブ内に含められるルート要素は次のものだけです[^api-yoake-mods][^yak-root][^yak-pack]。

| パス | 役割 |
| --- | --- |
| `yak.toml` | 必須のマニフェスト（ルート直下）[^yak-root][^yak-pack]。 |
| `LICENSE` / `README.md` | 任意のルートメタデータファイル[^api-yoake-mods][^yak-root]。 |
| `bin/wasm/<name>.wasm` | `[wasm]` で宣言したWebAssemblyコンポーネント（`bin/wasm/` 直下の `.wasm` ファイルのみ。サブフォルダは不可）[^wasm-dev][^yak-manifest][^yak-root]。 |
| `content/` | 展開済みの `.hmod` コンテンツ（`horizon-mod.toml` を含む）[^api-yoake-mods][^yak-root]。 |
| `assets/` | アセットファイル群[^api-yoake-mods][^yak-root]。 |
| `shaders/<id>/` | `[[shaders]]` で宣言した各シェーダーパックのファイル群[^api-yoake-mods][^yak-root]。 |
| `config/` | 設定関連ファイル群[^api-yoake-mods][^yak-root]。 |
| `bin/<target>/` | `[bin]`（ネイティブ MOD。上級の道）で宣言したターゲット別動的ライブラリ[^api-yoake-mods][^yak-root]。 |
| `SIGNATURE` | 署名時にルート直下へ追加されるUTF-8 JSONの署名ファイル[^api-yoake-mods][^yak-root]。 |

WebAssembly MOD の場合、`yak.toml` で次のように `bin/wasm/` 下の `.wasm` と `interface = "1.0"` を宣言します[^wasm-dev]。

```toml
[wasm]
client = "bin/wasm/hello.wasm"
interface = "1.0"
```

フルカタログ（client-full / server-full）を使う場合は次のように指定します[^wasm-dev]。

```toml
[wasm]
interface = "1.0"
client = "bin/wasm/demo-client.wasm"   # world client-mod (default) ...
world  = "client-full"                  # ... or the full client world
server = "bin/wasm/demo-server.wasm"   # always server-full
```

Mixin（`inject-mod`）の場合は `interface = "1.1"` と `world = "inject-mod"` を指定します（[8. Yoake Mixin 入門（`08-mixin.md`）](./08-mixin.md) を参照）。

### サイズと件数の上限

`.yak` のリーダーとインストーラーは以下の上限を検査します[^api-yoake-mods][^yak-manifest][^yak-root][^yak-installed]。

- **`.yak` アーカイブ全体のサイズ**: 最大 `1 GiB + 32 MiB`（`MAX_ARCHIVE_SIZE`）[^api-yoake-mods][^yak-root]
- **展開後の合計サイズ**: 最大 `1 GiB`（`MAX_TOTAL_SIZE`）[^api-yoake-mods][^yak-root]
- **通常ファイル1個あたりの展開後サイズ**: 最大 `256 MiB`（`MAX_FILE_SIZE`）[^api-yoake-mods][^yak-root]
- **WebAssemblyモジュール（`.wasm`）1個あたり**: 最大 `32 MiB`（`MAX_WASM_SIZE`）[^yak-manifest][^yak-root]
- **`yak.toml`**: 最大 `1 MiB`（`MAX_MANIFEST_SIZE`）[^api-yoake-mods][^yak-manifest]
- **`SIGNATURE`**: 最大 `2 MiB`（`MAX_SIGNATURE_SIZE`）[^api-yoake-mods][^yak-root]
- **ZIPエントリ数**: 最大 `16,384` 件、セントラルディレクトリ最大 `32 MiB`、パス長最大 `1,024` UTF-8バイト、最大圧縮率 `200:1`、展開先のディレクトリ階層最大 `64` 段[^api-yoake-mods][^yak-root][^yak-installed]

### `.yak` で拒否されるもの・入れてはいけないもの

次のものが1つでも含まれていると、パッケージ検証や展開で拒否されます[^api-yoake-mods][^yak-manifest][^yak-root][^yak-pack]。

- 宣言されたルート（`yak.toml`、`SIGNATURE`、`LICENSE`、`README.md`、`content/`、`assets/`、`shaders/`、`config/`、`bin/`）以外のファイルやフォルダ[^api-yoake-mods][^yak-root]。
- `yak.toml` の `[wasm]` で宣言していない `.wasm` ファイル、`[[shaders]]` で宣言していない `shaders/<id>/`、または `[bin]` で宣言していないターゲットのバイナリ[^yak-root]。
- **ネイティブコード（`[bin]`）とWebAssembly（`[wasm]`）の両方**: 1つのパッケージに `[bin]` と `[wasm]` を同時に入れることはできません[^yak-manifest]。
- 暗号化されたZIPエントリ、シンボリックリンク、Windowsリパースポイント、デバイスファイル・特殊ファイル[^api-yoake-mods][^yak-root][^yak-installed]。
- 不正なUTF-8、絶対パス、`..` や `.`、バックスラッシュ（`\`）、制御文字、空のパス要素、末尾のドットや空白、Windowsの予約名（`CON`、`PRN`、`AUX`、`NUL`、`COM1`〜`COM9`、`LPT1`〜`LPT9`）、重複パスや大文字・小文字だけが異なる衝突パス[^api-yoake-mods][^yak-manifest][^yak-root]。
- `yak.toml` の `[[services]]` におけるシークレットらしきキー名（`secret`、`token`、`password`、`apikey`、`privatekey` など）や、認証情報・クエリ・フラグメントを含むURL[^api-yoake-mods][^yak-manifest]。

> [!WARNING]
> **機密情報の混入禁止**: 上記の `[[services]]` キー名やURLの検査は、通常の文字列やファイル内に隠された秘密を汎用的に検出するものではありません[^api-yoake-mods]。パスワード、APIキー、トークン、秘密鍵などのシークレットは、パッケージやカタログのどこにも絶対に入れないでください[^api-yoake-mods]。

---

## 7-3. 署名とは何か・誰が署名するか

### パッケージ署名とカタログ署名の仕組み

Yoakeの署名には、`horizon-release-sign` による **Ed25519**（公開鍵は32バイトの標準Base64、署名は64バイトの標準Base64）が使われます[^api-yoake-mods]。署名の対象（ドメイン）は2種類あります[^api-yoake-mods]。

| ドメイン | プレフィックス | 署名対象と保存先 |
| --- | --- | --- |
| `yoake-yak-v1` | `horizon release yoake-yak-v1\n` | `SIGNATURE` を除く全通常ファイルのSHA-256ハッシュ一覧（パス昇順、`<64桁小文字hex>  <path>\n`）。ZIP内の `SIGNATURE`（JSON）として保存[^api-yoake-mods]。 |
| `yoake-mods-v1` | `horizon release yoake-mods-v1\n` | `index.json` の生バイト列。分離署名 `index.json.sig` として保存[^api-yoake-mods]。 |

署名・検証を行うリリースコマンドは次の通りです[^api-yoake-mods]。

```text
horizon-release-sign sign --tag yoake-yak-v1 --key-file K path/to/example_program-1.0.0.yak
horizon-release-sign verify --tag yoake-yak-v1 --key <base64-public-key> path/to/example_program-1.0.0.yak
horizon-release-sign sign --tag yoake-mods-v1 --key-file K path/to/index.json
horizon-release-sign verify --tag yoake-mods-v1 --key <base64-public-key> path/to/index.json
```

カタログの `size` と `sha256` は `SIGNATURE` を含むZIP全体のバイト列を対象とするため、**必ずパッケージに署名してから**カタログの `size` / `sha256` を記録します[^api-yoake-mods]。

### 誰が署名するか

- パッケージ内の `SIGNATURE` に書かれた公開鍵（`signer`）自体が信頼の根拠になることは**決してありません**[^api-yoake-mods]。
- ゲームクライアントとランチャーはどちらも、ビルド時に環境変数 `YOAKE_COMPILED_PUBLIC_KEYS`（`;` 区切り）から埋め込まれた **ピン留めされた Yoake の署名鍵（`horizon_mods::yak::trusted_keys()`）** だけを信頼します（ランチャー自身の更新やフィード検証と同じピン留めされた Yoake の署名鍵です）[^api-yoake-mods][^yak-installed][^launcher-mods]。この鍵を持たないビルドはどの鍵も信頼せず、フェイルクローズ（拒否）になります[^api-yoake-mods][^yak-installed]。
- 本番の秘密鍵はリポジトリ外の非公開フォルダに保管され、署名マシン上で `--key-file` により与えられます[^api-yoake-mods]。

> [!IMPORTANT]
> **プレイヤーに配る WebAssembly MOD には、Yoake の署名鍵による署名が必須です。**
> `yoake pack`（や `cargo horizon-mod pack`）が出力した `.yak` は**未署名（unsigned）**です[^api-yoake-mods][^yoake-cli]。未署名の `.yak` は、作者自身のマシンで Wasm の開発者モード（`YOAKE_WASM_UNSIGNED=1`、またはランチャーの「自分の WASM MOD をテスト」）を有効にしたときにしかゲーム内で動作しません[^yak-mods-client]。他のプレイヤーが通常環境で WebAssembly MOD を動かすには、ピン留めされた Yoake の署名鍵による有効な `SIGNATURE` がパッケージに入っている必要があります[^yak-mods-client][^yak-installed]。なお、MOD 作者がパッケージを提出して Yoake の署名を受ける具体的な手順・窓口は（未確定）です。

<!-- TODO(Yoake): 署名の依頼窓口とポータルへの提出手順を決めたら、ここと 07-x（ポータル申請・更新提出の（未確定）箇所）を書き換える -->

### ランチャーとゲームが検査すること

1. **ランチャーの検査**:
   - ポータルから取得した `index.json` を `index.json.sig` とピン留めされた Yoake の署名鍵（`trusted_keys()`）で検証し（`Catalog::parse_verified`）、各 MOD のダウンロードURLがポータルと同じHTTPSオリジンであることを確認します[^api-yoake-mods][^launcher-mods]。
   - ダウンロードした `.yak` のバイト数・全体のSHA-256・ZIP構造・正規化された `yak.toml` がカタログの記述と一致するか検証します（`Archive::from_catalog`）[^api-yoake-mods][^launcher-mods]。
   - `.yak` に `SIGNATURE` が入っている場合は、必ず `trusted_keys()` で検証します。壊れた署名や信頼されていない鍵の署名は、開発者モードであっても常に拒否されます[^api-yoake-mods][^launcher-plan][^launcher-local-tests]。
   - ランチャーのインストール計画（`plan.rs`）が未署名パッケージをブロックするのは `[bin]`（ネイティブ MOD）のみであるため、ランチャー自体はカタログ上またはローカルの未署名非 `[bin]` パッケージ（データ専用や `[wasm]`）をインストールできます（ローカルの場合は確認ダイアログ付きで `Unsigned .yak` として表示）[^launcher-plan][^launcher-rows]。**ただし、`[wasm]` パッケージにピン留めされた Yoake の署名鍵による有効な `SIGNATURE` がない場合、インストールできてもゲーム本体で拒否されて動きません**[^yak-mods-client]。
2. **ゲーム起動時の検査**:
   - クライアントは `<game dir>/yoake-mods.txt` と展開済みの `<game dir>/yoake-mods/<id>/<version>/yak.toml` だけを読みます（`.cache/` やカタログは読みません）[^api-yoake-mods]。
   - パスの安全性（シンボリックリンクやリパースポイントの拒否、サイズ上限）、Yoake本体のSemVer要件、サイドを検査します[^api-yoake-mods][^yak-mods-client]。
   - **WebAssemblyモジュール（`[wasm]`）のロード前検査**: ゲームは `wasm_mods_from` に、ピン留めされた Yoake の署名鍵（`yak::trusted_keys()`）と、環境変数から決まる未署名の許可（`UnsignedSwitches` の `wasm`）を渡して `wasm_preflight` を呼び出し、ツリーと `yak.toml`、`.wasm` ファイルサイズ（32 MiB以下）を確認したうえで、`yak::verify_installed` により `SIGNATURE`（ピン留めされた Yoake の署名鍵の署名と `yak.toml` および `.wasm` のSHA-256ハッシュ）を検証します[^yak-mods-client][^yak-installed]。`SIGNATURE` がない場合は、`YOAKE_WASM_UNSIGNED=1`（上位互換の `YOAKE_DLL_MODS_UNSIGNED=1` でも可）でない限りそのモジュールは拒否されます（`"YAK <id> wasm module refused"`）。`SIGNATURE` がある場合は、スイッチがあっても検証され、失敗すれば拒否されます[^yak-mods-client]。
   - **ネイティブライブラリ（`[bin]`）のロード前検査**: 直前にツリーと `yak.toml` と互換性を再確認し、DLLと `yak.toml` のハッシュをピン留めされた Yoake の署名鍵で認証された `SIGNATURE` のファイルマップと照合します（`verify_installed`）。署名のない DLL は、DLL 用の開発者モード（`YOAKE_DLL_MODS_UNSIGNED=1`）でだけ動きます[^api-yoake-mods]。

### 「未署名」とは何か・なぜ開発専用なのか

- `SIGNATURE` を持たないアーカイブは「未署名」です。`cargo horizon-mod yak verify` でZIP構造チェックに通っても、未署名のパッケージは認証されたことにはなりません[^api-yoake-mods]。
- 署名のない MOD を開発で動かすスイッチは、**Wasm 用と DLL 用の 2 つあり、別々に切り替えます**[^api-yoake-mods][^wasm-dev][^launcher-dev]。

  | | Wasm 用 | DLL 用 |
  | --- | --- | --- |
  | ランチャーの設定（"Launcher x.y.z" を 7 回クリックして出す "Developer options"） | 「自分の WASM MOD をテスト」（設定 `dev_unsigned_wasm`。確認画面で「オンにする」） | 「自作の DLL MOD を試す（開発者モード）」（設定 `dev_unsigned_dll`） |
  | 環境変数（値は正確に `1`） | `YOAKE_WASM_UNSIGNED` | `YOAKE_DLL_MODS_UNSIGNED` |
  | 許可するもの | `[wasm]` で `[bin]` のない、`SIGNATURE` の無い `.yak`、`YOAKE_WASM_DEV_DIR` の生 `.wasm` | Wasm 用の機能すべて（上位互換）に加えて、署名のないネイティブ DLL |

- Wasm モジュールについては、どちらのスイッチでも、`SIGNATURE` が付いているパッケージは検証されます。壊れた署名・信頼されていない鍵の署名・チェックサムの不一致は、開発者モードでも拒否されます。一方、DLL 用のスイッチはネイティブ DLL に限り、署名の欠落・不正・信頼されていない鍵・チェックサムの不一致をすべて見逃します（ネイティブ開発専用）[^api-yoake-mods][^yak-mods-client]。
- ホットリロードの対象は `client-mod` のモジュールだけです（未署名パッケージ内の `client-mod` モジュールと、`YOAKE_WASM_DEV_DIR` の開発フォルダ）。`client-full` / `server-full` は監視しません。ゲームは開発者モードで未署名のモジュールを監視対象にマークしますが（`watch: (developer && !module.signed)`、1秒おきにポーリング）、実際に監視を登録するのは `client-mod` のモジュールだけです[^wasm-dev][^wasm-mods-client]。`YOAKE_WASM_DEV_DIR=<folder>` からの `<id>.wasm` 直接読み込みもこのスイッチで有効になり、ID に `dev-` が付きます[^wasm-dev][^wasm-mods-client]。これらは自分の手元で開発・テストするための機能であり、プレイヤーへの配布に使うものではありません[^api-yoake-mods][^wasm-dev][^launcher-dev]。

---

## 7-4. ランチャーの MOD カタログへの載せ方

### フィード・ポータル・カタログの仕組み

ランチャーは 2 つの署名付きインデックスを使って環境と MOD を管理します。

1. **ベース環境フィード（`yoake_feed.rs`）**: ビルド時に `YOAKE_COMPILED_FEED_URL`、`YOAKE_COMPILED_PUBLIC_KEYS`、`YOAKE_COMPILED_SOURCE_ID`（スキーマ2では `"yoake-official"`）から構成され、`yoake-index-v1` ドメインで署名されます[^launcher-feed]。インスタンスに記録された署名済みの `YoakeBuildIdentity`（`version`、`engine`、`abi`）が、MOD の互換性チェックの基準になります[^launcher-mods][^launcher-feed]。
2. **MOD カタログ（`index.json`）**: ビルド時に設定されたポータル（`YOAKE_PORTAL_URL`）の `/yoake/mods/index.json`（最大16 MiB）と分離署名 `/yoake/mods/index.json.sig`（最大4 KiB、`yoake-mods-v1` ドメイン）から取得されます[^api-yoake-mods][^launcher-mods]。各パッケージの配布URLは `https://<portal>/yoake/mods/<id>/<version>/<id>-<version>.yak` に固定されています[^api-yoake-mods][^yak-catalog]。
   - カタログとパッケージの取得にはポータルのベアラートークン認証（メンバーのブラウザデバイスログイン、またはフレンドのテストプレイ招待コードで取得した `yoake-token`）が必要です[^launcher-mods][^launcher-login]。テストプレイ招待で参加するプレイヤーも同じカタログのパスを使います[^api-yoake-mods]。

> [!NOTE]
> `docs/api/yoake-mods.md` に「ここのコマンドはファイル公開や上流インデックスの編集を行わない」と明記されている通り[^api-yoake-mods]、MOD 作者がポータルへパッケージを申請・登録する具体的な手順やポータルの実URLは、参照元のソースには記載されていません（未確定）。

### `yak.toml` と `index.json` に必要なメタデータ

カタログに掲載されるには、`yak.toml` の全フィールドが検証を通過し、その正規化されたJSON表現が `index.json` の `manifest` に入る必要があります[^api-yoake-mods][^yak-catalog]。

- **`yak.toml` の必須項目**[^yak-manifest]:
  - `format = 1`（未知のフィールドはすべてエラーとして拒否されます）
  - `id`（パッケージ ID）: `3〜48` 文字のASCII小文字・数字・`_`・`-`（Windows予約名は不可）
  - `version`: 完全なSemVer文字列（例: `"1.0.0"`）
  - `name`: 表示名（`1〜256` バイト、制御文字不可）
  - `authors`: 作者名の配列（`1〜128` 件、各 `1〜256` バイト）
  - `description`: 説明文（`1〜16,384` バイト）
  - `[yoake] version`: 対応するYoake本体のSemVer範囲（例: `">=0.3.0, <0.4.0"`）。`yoake new` が作る `version = "*"` は開発用です。公開する `.yak` では `">=0.3.0, <0.4.0"` のように範囲を絞ってください
  - `[side]`: `client`、`server`、`required_on_join` の真偽値（ランチャーでクライアントインスタンスに入れる MOD は `client = true` が必要です[^launcher-plan]）
- **`yak.toml` の主な任意項目**[^yak-manifest][^wasm-dev]:
  - `name_ja`（日本語UIで優先表示される名前、`1〜256` バイト[^launcher-mods]）、`license`（`1〜256` バイト）、`homepage`（認証情報・クエリ・フラグメントを含まないHTTP/HTTPS URL、最大 `4,096` バイト）
  - `dependencies`、`optional`、`conflicts`（各最大 `1,024` 件、自己参照不可、`dependencies` と `optional` の重複不可）
  - `[wasm]`: `interface = "1.0"`、`client` / `server` の `.wasm` パス、`world`（`"client-mod"`、`"client-full"`、`"inject-mod"`）、`permissions`（`"service.chat"`、`"service.commands"`、`"service.config"`、`"service.http"`、`"service.permissions"`、`"service.storage"` の 6 つから選択）。`interface` は `"1.0"` で、`inject-mod` だけ `"1.1"` です
  - `[mixin]`（`inject-mod` 用の `targets`）、`[config.<key>]`（最大 `256` キー）、`[permissions."<node>"]`（最大 `256` ノード）、`[http]`（`hosts`、`service.http` サービス権限が必要）、`[[shaders]]`（最大 `128` 個）、`[[services]]`（最大 `128` 個）
- **`index.json` の構造**（`docs/api/yoake-mods.md` より）[^api-yoake-mods]:

```json
{
  "schema": 1,
  "generated": "2026-11-13T00:00:00Z",
  "entries": [{
    "id": "example_data",
    "versions": [{
      "version": "1.0.0",
      "url": "https://portal.example/yoake/mods/example_data/1.0.0/example_data-1.0.0.yak",
      "size": 1024,
      "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
      "manifest": {
        "format": 1,
        "id": "example_data",
        "version": "1.0.0",
        "name": "Example data",
        "authors": ["Example author"],
        "description": "An example data-only package.",
        "license": "MIT",
        "yoake": { "version": ">=0.3.0, <0.4.0" },
        "side": { "client": true, "server": true, "required_on_join": false }
      },
      "targets": [],
      "engine": null
    }]
  }]
}
```

---

## 7-5. 更新の出し方と互換性

### 新しいバージョンを出す手順とランチャーでの見え方

1. `yak.toml`（および `content/horizon-mod.toml` がある場合はその両方）の `version` を前より大きいSemVerに上げ、必要なら `[yoake] version` などの要件を更新して `.yak` をパックします[^api-yoake-mods][^yak-manifest]。
2. パッケージに署名したあと、その最終ZIPバイト列の `size` と `sha256` を計算し、`index.json` の該当 MOD の `versions` 配列に新しいバージョンを追加します[^api-yoake-mods]。`index.json` は一度だけシリアライズしてそのまま `yoake-mods-v1` で署名します（署名後に空白や改行を1文字でも変えると署名が無効になります）[^api-yoake-mods]。なお、作者が更新をポータルへ提出する手順は（未確定）です。
3. ランチャーは MOD 画面を開いたとき、手動更新（`Refresh`）したとき、認証情報が変わったとき、または **300秒（5分）おき** にカタログを再取得します[^launcher-page]。
4. ランチャーはインストール済み MOD と同じ `id` のカタログバージョンのうち、**現在のインスタンス環境と互換性があり（`environment.check(&version.manifest).is_ok()`）、かつ `version > installed.version` である最大のSemVer** を探します[^launcher-plan][^launcher-rows]。見つかると MOD 一覧の行に `Update <version>` ボタンが表示され、プレイヤーが確認画面（`Review mod changes` → `Confirm`）で承認すると、新しいバージョンが `<game dir>/yoake-mods/<id>/<new-version>/` に展開されて `<game dir>/yoake-mods/<id>/<old-version>/` が削除され、`installed.json` と `yoake-mods.txt` が依存順で更新されます[^launcher-page][^launcher-rows][^launcher-tx]。

### 互換性のルール（`[yoake] version` と `interface`）

- **`[yoake] version`**: インスタンスのYoakeバージョンが `yak.toml` の `[yoake] version`（SemVer範囲）に一致するかどうかが `check_data_compatibility` で判定されます[^api-yoake-mods][^yak-compat]。Yoake本体が更新されて範囲外になった MOD は、ランチャーで互換性エラーとして表示され、クライアント起動時にもスキップされてタイトル画面に通知が出ます[^api-yoake-mods][^launcher-mods]。
- **`[wasm] interface`**: `"major.minor"` 形式で指定し、現在サポートされているのは `interface = "1.0"` と、`world = "inject-mod"` のときだけ使える `"1.1"`（`SUPPORTED_WASM_INTERFACE = (1, 1)`）です（メジャー番号が `1` 以外、またはマイナー番号が `1` より大きいものは拒否されます）[^wasm-dev][^yak-manifest]。また、`server-full` の WIT ワールドに `handle-call` エクスポートが追加されたときのようにホスト側の WIT ワールドが変わった場合、変更前にビルドされたモジュールはロードできないため、新しいSDKで再ビルドする必要があります[^wasm-dev]。

---

## 7-6. やってはいけないこと

ソースコードと仕様書で裏付けられている、MOD 公開時の禁止事項です。

1. **プレイヤーに `YOAKE_WASM_UNSIGNED=1` の設定やランチャーの「自分の WASM MOD をテスト」の有効化を求めない**:
   これらは署名検証をバイパスする開発専用の仕組みです。Wasm はサンドボックスで動きますが、宣言されたサービス権限（`service.commands` など）は有効です。DLL 用のスイッチ（`YOAKE_DLL_MODS_UNSIGNED=1`、「自作の DLL MOD を試す（開発者モード）」）も同様で、ネイティブ DLL コードはサンドボックス化されていません[^api-yoake-mods][^launcher-dev]。配布する WebAssembly MOD は必ず Yoake の鍵で署名されていなければならず、未署名のままではプレイヤーのゲーム環境で実行できません[^yak-mods-client]。
2. **WebAssembly MOD にネイティブ DLL を混ぜたり、未署名の `[wasm]`・`[bin]` MOD をプレイヤーに配布したりしない**:
   `.yak` に `[bin]` と `[wasm]` を同時に入れることは禁止されており、宣言していないバイナリも拒否されます[^yak-manifest][^yak-root]。またゲーム本体は `[wasm]` と `[bin]` の両方に対して Yoake のピン留めされた Yoake の署名鍵による有効な `SIGNATURE` を要求するため、未署名のコード MOD をプレイヤー向けに配布してはいけません[^api-yoake-mods][^yak-mods-client][^launcher-plan]。
3. **他の MOD のパッケージ ID（`yak.toml` の `id`）を使い回さない**:
   カタログ、`installed.json`、`yoake-mods.txt`、依存解決（`yak::resolve`）はすべて `id` が一意であることを前提としており、同じパッケージ ID の MOD を入れると既存の MOD が置き換えられます[^api-yoake-mods][^yak-catalog][^launcher-tx]。さらに設定ファイル（`<game>/config/<id>.toml`）、ストレージ（`yoake-data/<id>/...`）、権限設定（`[mods.<id>]`）もすべてパッケージ ID ごとに分離されています[^wasm-dev][^wasm-storage]。
4. **パッケージや `yak.toml`、カタログに秘密鍵・トークン・パスワードを入れない**:
   `yak.toml` の `[[services]]` やURLではトークン・パスワード・秘密鍵・APIキー・認証情報らしきキーやURL内資格情報が拒否されますが、通常の文字列に隠された秘密を万能に検出するものではありません。パッケージやカタログ、リポジトリのどこにもシークレットや署名用の秘密鍵を入れてはいけません[^api-yoake-mods][^yak-manifest]。
5. **ストレージの内部ファイル形式に依存したり、ファイルを直接編集・`std::fs` で読み書きしようとしたりしない**:
   WebAssemblyゲストには事前オープンされたディレクトリがなく、`std::fs` の呼び出しはすべて失敗します[^wasm-dev]。またディスク上のファイル形式はホスト内部の実装であり将来変更される可能性があるため、ファイルを直接編集せず、必ず `storage_get` / `storage_put` / `storage_delete` および `storage_blob_*`（WITの `storage-get` / `storage-put` / `storage-delete` / `storage-blob-*` インポート）だけを使ってください[^wasm-dev][^wasm-storage]。ストレージの仕様（スコープ別フォルダ、容量上限、書き込み予算、旧形式の取り込み）は [4 章の 4-2](./04-examples.md) にまとめてあります。
6. **署名後の `.yak` や `index.json` を書き換えたり、展開先フォルダ（`<game dir>/yoake-mods/<id>/<version>/`）を再パックしたりしない**:
   署名後に1バイトでも書き換えると署名やSHA-256が無効になります。また展開先フォルダに残される `SIGNATURE` は「元の完全な `.yak` アーカイブ」に対する署名であり、展開されたサブセットを単体の署名済みアーカイブとして再パック・検証することはできません[^api-yoake-mods]。

---

## 次のステップ

公開の流れが分かったら、次は [8. Yoake Mixin 入門（`08-mixin.md`）](./08-mixin.md) に進んで、エンジン関数に割り込む Mixin の書き方を見ていきましょう。目次に戻る場合は [README](./README.md) を開いてください。

[^api-yoake-mods]: 確認元: `docs/api/yoake-mods.md`
[^wasm-dev]: 確認元: `docs/yoake-wasm-dev.md`
[^wasm-storage]: 確認元: `crates/yoake-wasm/src/host_services/storage.rs`
[^yoake-cli]: 確認元: [`02-first-mod.md`](./02-first-mod.md)、[`03-play.md`](./03-play.md)、`crates/yoake-cli/src/main.rs`（`yoake pack`）
[^yak-root]: 確認元: `crates/horizon-mods/src/yak.rs`
[^yak-manifest]: 確認元: `crates/horizon-mods/src/yak/manifest.rs`
[^yak-catalog]: 確認元: `crates/horizon-mods/src/yak/catalog.rs`
[^yak-compat]: 確認元: `crates/horizon-mods/src/yak/compatibility.rs`
[^yak-installed]: 確認元: `crates/horizon-mods/src/yak/installed.rs`
[^yak-pack]: 確認元: `crates/horizon-mods/src/yak/pack.rs`
[^yak-mods-client]: 確認元: `crates/horizon-client/src/yak_mods.rs`
[^wasm-mods-client]: 確認元: `crates/horizon-client/src/wasm_mods.rs`
[^launcher-mods]: 確認元: `launcher/src/yoake_mods.rs`
[^launcher-plan]: 確認元: `launcher/src/yoake_mods/plan.rs`
[^launcher-tx]: 確認元: `launcher/src/yoake_mods/plan/transaction.rs`
[^launcher-local-tests]: 確認元: `launcher/src/yoake_mods/plan/tests/local.rs`
[^launcher-page]: 確認元: `launcher/src/yoake_mods_page.rs`
[^launcher-rows]: 確認元: `launcher/src/yoake_mods_page/rows.rs`
[^launcher-feed]: 確認元: `launcher/src/yoake_feed.rs`
[^launcher-login]: 確認元: `launcher/src/yoake_login.rs`
[^launcher-dev]: 確認元: `launcher/src/dev_options.rs`
