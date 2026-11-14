# .yak — Yoake の MOD を 1 ファイルにする形式（仕様 v1 案）

2026-11-12、Claude。ユーザーの案（「アイテム追加しようがシェーダー追加しようがひとつのファイルでいい」）を形にしたもの。
`YOAKE_SURPASS.md` の S1 3 段目（署名・ランチャー配布・マルチの照合）を、この形式で作る。

## 1. 何を決めるか
- 1 つの MOD を 1 つの `.yak` ファイルで配る。中身は **zip**（Deflate か無圧縮。暗号化なし、zip64 可）。
- プログラム（DLL）・コンテンツ（今の `.hmod`）・画像や音・シェーダーパック・設定の初期値を、同じファイルに入れられる。どれも省ける。
- ランチャーが入れて、ゲームが読む。遊ぶ人がファイルを手で扱わなくてよい（ランチャーの外で受け渡すこともできる）。
- 今の `.hmod`・`.hnmod`・`.hshader` は、そのまま読める。`.yak` はそれらを包む上の形。

## 2. 中の並び
```
mymod.yak
├─ yak.toml                      必須。下の 3 章
├─ bin/                          プログラム入りの MOD だけ
│  ├─ windows-x64/mymod.dll
│  ├─ linux-x64/libmymod.so
│  └─ macos-arm64/libmymod.dylib
├─ content/                      今の .hmod の中身そのまま（horizon-mod.toml・data/・assets/）
├─ assets/                       MOD のプログラムが読む画像・モデル・音・フォント（content を通さないもの）
├─ shaders/<pack-id>/            今の .hshader の中身そのまま（1 つの .yak に複数可）
├─ config/
│  ├─ defaults.toml              設定の初期値
│  └─ settings.json              設定画面の定義（yoake-kit の SettingsGroup の形）
├─ LICENSE / README.md           任意
└─ SIGNATURE                     署名（5 章）。yak.toml と各ファイルのハッシュの一覧への署名
```
- パスは小文字推奨・`/` 区切り・`..` や絶対パスは禁止（入れる前に弾く）。シンボリックリンク禁止。
- 1 ファイル 256 MB まで、全体 1 GB まで（ランチャーが入れる前に確かめる）。
- MOD の中で使うシェーダー（WGSL）は DLL に埋め込んでよい。`shaders/` はシェーダーパック（Kage など）用。

## 3. yak.toml
```toml
format = 1                       # この仕様の版
id = "webscreen"                 # a-z 0-9 _ - 、3〜48 字。ほかと重ならない
version = "0.3.1"                # semver
name = "WebScreen"
name_ja = "ウェブスクリーン"       # 任意
authors = ["..."]
description = "..."
license = "MIT"                  # 任意
homepage = "https://..."         # 任意

[yoake]
# データだけの MOD はこの範囲で動く。bin/ がある MOD は 4 章の engine も見る
version = ">=0.3, <0.4"

[side]
client = true                    # クライアントで要る
server = false                   # サーバー（統合サーバーの持ち主・専用サーバー）で要る
required_on_join = false         # 参加する人も同じものを持っている必要があるか（マルチの照合に入る）

[dependencies]                   # ほかの .yak。semver の範囲
yoake-ui = "^0.1"
[optional]                       # あれば使う（yoake-map のマーカー API など）
yoake-map = "^0.1"
[conflicts]                      # 一緒に入れられないもの
# other-mod = "*"

[bin]                            # bin/ がある時だけ
library = "webscreen"            # bin/<target>/ の中のファイル名の元（dll / lib*.so / lib*.dylib）
targets = ["windows-x64"]        # 入っている target
engine = "0.3.1+build.ef3e041fa" # この DLL を組んだ Yoake（エンジン）の版。合わないとランチャーが入れない
abi = 2                          # horizon-mod-abi の版

[[shaders]]                      # shaders/ がある時だけ
id = "kage"
name = "Kage"
# パックが選択肢を宣言し、遊ぶ人がシェーダー設定で選ぶ（Kage と Kage Lite を 1 つにする）
[shaders.options.shadow_map]
values = [1024, 2048]
default = 2048
label_ja = "影の細かさ"

[[services]]                     # 外のサービスが要る時だけ（Mimic の AI サーバーなど）
id = "mimic-brain"
name = "Mimic Brain"
required = false                 # 無くても MOD は動くか
default_url = "http://127.0.0.1:8790"
help_url = "https://..."         # 用意のしかたの説明
# 鍵・トークンなどの秘密は書かない。ランチャーは案内するだけで、入れたり起動したりしない
```

### WebAssembly code (interface 1.0)

DLL の代わりに WebAssembly（`yoake:mods` インターフェース）でコードを持てる。`[bin]` と `[wasm]` は同じ .yak に同居できない（1 パッケージにつきコードの種類は 1 つ）。

```toml
[wasm]
interface = "1.0"                  # [wasm] があれば必須。yoake:mods の major.minor
client = "bin/wasm/<name>.wasm"    # 任意。クライアント側
server = "bin/wasm/<name>.wasm"    # 任意。サーバー側（W2 用に予約。検証はするが、まだ読み込まない）
```

- `client` と `server` のどちらか 1 つは必要。
- パスは `bin/wasm/` の直下で、`.wasm` で終わり、`..` や バックスラッシュを含まない。アーカイブ内に実在し、1 つ 32 MiB 以下。宣言していない `bin/wasm/` のファイルは拒否される。
- `interface` の major は 1 のみ。minor はゲームが対応する最大（現在 1.0）以下。それ以外ははっきりしたエラーで拒否。
- `[wasm]` が無い既存の manifest はこれまでどおり有効で、何も変わらない。
- 署名の扱いは DLL と同じ：署名済み（カタログ署名）は読める。ローカルの未署名は開発者設定（`YOAKE_DLL_MODS_UNSIGNED=1`）のときだけ。
- `cargo horizon-mod pack` は宣言された `bin/wasm/*.wasm` をそのままパッケージに入れる（`--bin` は使わない）。マニフェスト層は wasm の中身を解析しない。

## 4. 版の合わせ方
- **データだけ（bin/ が無い）**：`[yoake].version` の範囲に入っていれば、Yoake の版が上がってもそのまま使える。
- **プログラム入り**：DLL は組んだエンジンと完全に一致する時だけ読む（`[bin].engine` とゲームの build fingerprint、`[bin].abi` と ABI 版）。合わなければランチャーが入れる前に止め、「この MOD は 0.3.2 用がまだありません」と出す。カタログに新しい版があれば自動で取り替える。
- target は `windows-x64`・`linux-x64`・`macos-arm64`・`macos-x64`。今の OS の target が無ければ入れない。

## 5. 署名
- `SIGNATURE` は JSON（実装に合わせた。形は `horizon/docs/api/yoake-mods.md`）。zip の中の全ファイルの SHA-256（`SIGNATURE` 自身を除く）と、その一覧への ed25519 署名（タグ `yoake-yak-v1`）。鍵はランチャーの更新と同じ Yoake の鍵で、ビルド時に `YOAKE_COMPILED_PUBLIC_KEYS` で埋め込む。
- **カタログ署名**（Yoake の配布元の鍵。今のランチャー更新と同じ仕組み。鍵は持ち主だけが持つ）：プログラム入りの `.yak` は、これが無いと読まない。
- **作者署名**（任意）：作者の公開鍵を `[signer]` に書く。カタログは「この id はこの作者の鍵」と覚え、別の鍵で同じ id が来たら止める。
- データだけの `.yak` は署名なしでも入れられる（リソースパックと同じ扱い）。ただし「署名なし」と表示する。
- 開発中は `YOAKE_DLL_MODS_UNSIGNED=1` で署名なしの DLL も読める（今と同じ）。ランチャーの設定で「開発者モード」として出す。

## 6. 入れ方と読み方
- ランチャーは `.yak` をインスタンスの `yoake-mods/<id>/<version>/` に展開する（`bin/` は今の OS の target だけ）。元の `.yak` は `yoake-mods/.cache/` に残す（共有コード・マルチの照合用）。
- ランチャーは `yoake-mods.txt`（有効な MOD の id と版、読む順）を書く。ゲームはこれを読む（今の環境変数 `YOAKE_DLL_MODS` の代わり。環境変数は開発用に残す）。
- 読む順：依存の順 → 同じ段は id の順。組み込み MOD → DLL MOD の順（今と同じ）。
- ゲームは `content/` を `.hmod` と同じ道で、`shaders/` を `.hshader` と同じ道で、`bin/` を DLL MOD として読む。
- マルチの照合：`required_on_join = true` の MOD の `id@version#sha256` を `horizon:native_mods` に入れる。足りない人には、ランチャーが「この MOD を入れて参加」を出す。

## 7. 作る道具
- `cargo horizon-mod pack [--target windows-x64 ...]`：cdylib を組み、`content/`・`assets/`・`shaders/`・`config/` を集めて `yak.toml` を確かめ、`.yak` を作る（署名なし）。
- `cargo horizon-mod yak verify <file>`：並び・パス・大きさ・ハッシュ・署名を確かめる。
- カタログへの登録と署名は、配布元（Claude/lead）が `horizon-release-sign` で行う。

## 8. 段階
1. この仕様を決める（今）。
2. 読み書きのクレート `horizon-yak`（zip・yak.toml・SIGNATURE・検証。エンジンにもランチャーにも入る）と `cargo horizon-mod pack` / `yak verify`。
3. ゲーム：`yoake-mods.txt` を読み、展開済みの `.yak` を読む。シェーダーの選択肢（影の大きさ）。
4. ランチャー：入れる・外す・更新・署名の確認・サービスの案内・マルチで足りない MOD の案内。S4 の MOD 選択・共有コードとつなぐ。
5. 既存の MOD を `.yak` にする：WebScreen・Mimic・replay・yasagashi・Kage・Nestgate のアドオン。

## 決めたこと（11/13 ユーザー）
- カタログと .yak の置き場所はポータルの `/yoake/`（今のランチャー更新と同じ。`/yoake/mods/index.json`（署名つき）と `/yoake/mods/<id>/<version>/<id>-<version>.yak`）。入口は今と同じ forward_auth（`/api/launcher/check`）なので、会員のトークンも招待コードのトークン（テストプレイ班）も同じように取れる。
- テストプレイ班も同じ仕組みで MOD を入れる（ランチャーの MOD 画面・共有コード・自動更新が同じに動く）。
- 作者署名は後回し。最初はカタログ署名だけで始める。
