# 1. 環境準備

Yoake の MOD は **Rust** というプログラミング言語で書き、**WebAssembly（`wasm32-wasip2`）** という形式に変換（ビルド）して動かします。
このページでは、パソコンに開発ツールを入れる手順を説明します。Windows を中心に説明し、macOS / Linux については最後に一言ずつ補足します。

---

## 1-1. Rust（rustup）をインストールする

まずは Rust の公式インストーラである **rustup** を入れます。このガイドの CLI（`yoake`）に合わせ、**Rust 1.95 以降**を用意してください。[^rust-version]

### Windows の場合

1. ブラウザで公式サイト（`https://rustup.rs/`）を開き、`rustup-init.exe` をダウンロードして実行します。
2. 画面の案内に従ってインストールを進めます（途中で Visual Studio C++ Build Tools の導入を案内された場合は、画面の指示通りにインストールしてください）。
3. インストールが終わったら、**PowerShell** または **コマンドプロンプト** を新しく開き直します。

### macOS / Linux の場合

- **macOS**: ターミナルを開き、公式サイト（`https://rustup.rs/`）の案内通りに `rustup` をインストールします（Xcode Command Line Tools がまだ無い場合はあわせて導入されます）。
- **Linux**: お使いのディストリビューションのターミナルから、公式サイト（`https://rustup.rs/`）の案内通りに `rustup` をインストールします。

---

## 1-2. WebAssembly ターゲット（`wasm32-wasip2`）を追加する

通常の Rust は今使っているパソコン向けのプログラムを作りますが、Yoake の MOD はゲーム内のサンドボックスで動く **WebAssembly コンポーネント（`wasm32-wasip2`）** としてビルドします。

PowerShell（macOS / Linux はターミナル）を開き、次のコマンドを 1 回だけ実行してください。

```sh
rustup target add wasm32-wasip2
```

- これだけで WebAssembly コンポーネントを出力できるようになります。
- **補足**: `wasm-tools` などの外部ツールを別途インストールする必要はありません。

PowerShell で環境変数を設定するときは、次の形式を使います。

```powershell
$env:NAME='value'
```

この設定は**その PowerShell ウィンドウだけ**に有効です。ウィンドウを閉じると失われます。

---

## 1-3. `yoake` CLI を入れる

`yoake` コマンド（次の章の `yoake new`、`yoake build` などで使うツール）は、このリポジトリの `crates/yoake-cli`（パッケージ名 `yoake`、Rust 1.95 以降）です。[^cli] 今のところは、公開されているリポジトリから入れます。リポジトリのルートフォルダで、次のコマンドを実行してください。

```sh
cargo install --path crates/yoake-cli
```

ビルドされた `yoake` の実行ファイルは Cargo の `bin` フォルダ（`~/.cargo/bin`）に置かれます。このフォルダは `rustup` がすでに `PATH` に追加しています。入れたあとは、ターミナルを新しく開いて `yoake` が動くか確かめてください。

1.0 では GitHub Releases に Windows 用の実行ファイルも置く予定です（未確定）。

---

## 1-4. エディタを用意する

MOD のプログラム（`.rs` ファイル）や設定ファイル（`yak.toml`、`Cargo.toml`）は、普通のテキストファイルです。使い慣れたテキストエディタがあればどれでも構いません。

迷ったら **Visual Studio Code（VS Code）** がおすすめです。

- 拡張機能の **rust-analyzer** を入れると、関数名やフック名の補完、書き間違いの指摘がエディタ上でリアルタイムに表示されるため、Rust が初めてでも格段に書きやすくなります。
- `yak.toml` や `Cargo.toml` を読みやすくする TOML 用の拡張機能（Even Better TOML など）もあると便利です。

---

## 1-5. 作業フォルダを決める

自分の MOD を作るための作業フォルダ（ワークスペース）を用意しましょう。

- Windows なら、たとえば `C:\Users\<あなたのユーザー名>\yoake-mods-work` のように、日本語や空白を含まない分かりやすいフォルダを作っておくとトラブルが少なくなります。
- macOS / Linux でも同様に、ホームディレクトリの下に `~/yoake-mods-work` などの作業用フォルダを作っておきます。
- MOD ごとにフォルダが 1 つ作られます。次の章で使う `yoake new` コマンドを実行すると、その作業フォルダの中に新しい MOD 用のフォルダが一式生成されます。

> [!NOTE]
> Windows では `con`、`prn`、`aux`、`nul`、`com1`〜`com9`、`lpt1`〜`lpt9` といった名前は OS の予約語のため、MOD 名やフォルダ名には使えません（`yoake new` でも拒否されます）。`my-hud` のような英小文字・数字・ハイフン（`-`）の名前をつけましょう。

---

## 次のステップ

準備ができたら、次は [2. 最初の MOD を作る（`02-first-mod.md`）](./02-first-mod.md) に進んで、実際に最初の HUD MOD を作成・ビルドしてみましょう。

[^rust-version]: ワークスペース共通の `rust-version` は 1.85 ですが、CLI（`crates/yoake-cli`）と、それが使う `horizon-mods` の `rust-version` は 1.95 です。確認元: `Cargo.toml`、`crates/yoake-cli/Cargo.toml`、`crates/horizon-mods/Cargo.toml`。
[^cli]: 確認元: `crates/yoake-cli/Cargo.toml`（`name = "yoake"`）。
