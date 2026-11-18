# フルアクセス MOD のひな形

English: [README.md](README.md)

Yoake 2.0 のフルアクセス MOD の最小サンプルです。サーバーの tick ごとにエンジン関数を呼び出し（`Game.experience` のハンドルを開いて `Experience.total` を読む）、そのあと本来の tick を実行させます。フルアクセス MOD とは何か、プレイヤー側で何が必要かは [`docs/yoake2/MOD-GUIDE.ja.md`](../../docs/yoake2/MOD-GUIDE.ja.md) を読んでください。

## ファイル

| ファイル | 内容 |
|---|---|
| `yak.toml` | パッケージのマニフェスト（`world = "full-access"`、サーバー側モジュール 1 つ） |
| `src/lib.rs` | MOD 本体。`init` で `hooks::SERVER_TICK` を登録し、`hook` でエンジン関数を呼んで `original` を実行する |
| `build.sh`, `build.ps1` | `rustc` で SDK と MOD をビルドし、`dist/<id>-<version>.yak` にパックする |

## ビルド

Yoake のソース一式（このフォルダは `templates/full-access-mod` にあります）と、`wasm32-wasip2` ターゲット付きの rustc **1.100.0** が必要です。どちらもソース一式の `rust-toolchain.toml` が指定しています。

```sh
./build.sh          # Linux、macOS、Git Bash
```

```powershell
.\build.ps1         # Windows PowerShell
```

どちらのスクリプトも 3 段階で動きます。まず `crates/yoake-guest-sdk` を `wasm32-wasip2` 向けの rlib としてコンパイルします。次に `src/lib.rs` をそれに対してコンパイルし、`bin/wasm/my_full_access_mod.wasm`（コンポーネントではなくコアモジュール）を作ります。最後にソース一式にあるパックツール `cargo run -p cargo-horizon-mod -- pack` で固めます。できあがりは `dist/my_full_access_mod-0.1.0.yak` です。署名なしですが、自分の PC に入れるぶんには署名は要りません。

設定: `YOAKE_REPO`（ソース一式の場所。初期値は 2 つ上のフォルダ）、`YOAKE_TOOLCHAIN`（初期値は `1.100.0`）。

ひな形をソース一式の外にコピーして使うときは、`YOAKE_REPO` にソース一式の場所を指定してください。

GNU ツールチェーンの Windows では、パックツールのコンパイルに MinGW の `bin` フォルダ（`dlltool` と `as` が入っているもの）を `PATH` に通す必要があります。MOD 本体のビルドに必要なのは `rustc` だけです。

## 自分の MOD にする

1. ID を決めます（半角の英小文字・数字・`_`・`-` で 3〜48 文字）。`yak.toml` の `id` と `[wasm] server` のファイル名、両スクリプトの `NAME`（`build.ps1` では `$name`）を書き換えます。モジュールのファイルは `bin/wasm/` の直下に置き、拡張子は `.wasm` にします。
2. `name`、`authors`、`description` を直し、リリースのたびに `version` を上げます（プレイヤーは新しいファイルをもう一度許可する必要があります）。
3. `src/lib.rs` にフックを書きます。

## インストール

ランチャーの Mods ページで **Add a .yak file…**（またはドラッグ＆ドロップ）を選び、MOD を **Allowed**（許可）に切り替えて確認します。ランチャーはファイルの中身そのものを記録するので、作り直したファイルは改めて許可が必要です。許可したあとにランチャーからゲームを起動してください。

## ゲームなしで試す

`crates/horizon-client/tests/yoake2_template.rs` が、このひな形をビルドしてパックし、インストール・許可・読み込みまで本物のローダーで行い、仮の `Game` で tick を回します。

```sh
cargo test --offline -j 4 -p horizon-client --features yoake2-slice,yoake-hook-bench --test yoake2_template
```

`src/lib.rs` の `report(slot, value)` はテスト用の仕組みで、読むのはテストだけです。そのためひな形では最初の 3 tick でしか呼びません。
