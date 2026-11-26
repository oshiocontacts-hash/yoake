# 12. ファイル・ネットワーク・ワーカー

MOD からファイルやネットワークを使う方法です。Yoake の MOD は身内のグループの中で使います。そのため、Forge や Fabric の MOD と同じように、ファイルとネットワークが使えます。

守られている決まりは 2 つです。

- MOD を入れていないときは、ゲームは何も変わりません。
- ゲームのメインスレッドは、MOD を待ちません。

## ファイル

MOD には次のフォルダが見えます。

| MOD から見えるパス | 実際のフォルダ | メモ |
|---|---|---|
| `/data` | `<ゲームフォルダ>/mod-data/<MOD の id>/` | その MOD 専用。最初に自動で作られる。読み書きできる |
| `/game` | インスタンスのゲームフォルダ | 読み書きできる |
| `/host/<ドライブ文字>` | PC のドライブのルート。例: `/host/c` | `yak.toml` に `host_filesystem = true` と書いたときだけ |

保存したいデータは `/data` の下に、ふつうのファイルとして置きます。Rust の `std::fs` がそのまま使えます。

```rust
std::fs::write("/data/score.txt", "42")?;
let text = std::fs::read_to_string("/data/score.txt")?;
```

> [!NOTE]
> ファイルの呼び出しは、フックの時間に入ります。大きなファイルを読み書きするときは、あとで説明するワーカーを使ってください。

## ネットワーク

ネットワークは待ち時間が長くなることがあります。メインのインスタンスで使うと、ゲームが止まってしまいます。だから、**ワーカー**の中で使います。メインのインスタンスで接続やリクエストをしても、アクセス拒否のエラーになります。

## ワーカー

ワーカーは、MOD の裏で動くもう 1 つのコンポーネントです。専用のスレッドで動くので、長く待ってもゲームは止まりません。ワーカーでは次のものが使えます。

- ファイル（上と同じ `/data` など）
- TCP と UDP と名前解決（`std::net` など）
- HTTP の送信（`wasi:http`）
- 時間のかかる待ち（`thread::sleep` など）

ワーカーはゲームの中身（エンジンのハンドル）には触れません。MOD とのやり取りは、バイト列のメッセージだけです。

`yoake new worker my-worker` でサーバー＋ワーカーのひな形をすぐ作れます。`yak.toml` にワーカーのファイルを書きます。

```toml
[wasm]
interface = "1.7"
server = "bin/wasm/my-worker.wasm"
worker = "bin/wasm/my-worker-worker.wasm"
```

メッセージのやり取りは次のとおりです。

| 誰が | 送る | 受け取る |
|---|---|---|
| MOD | `worker::send`（`worker-send`） | `worker::recv`（`worker-recv`。待たずに、なければ `None`） |
| ワーカー | `worker::send`（`host-send`） | `worker::recv`（`host-recv`。来るまで待つ）、`worker::try_recv`（`host-try-recv`。待たない） |

SDK の `yoake_wasm_sdk::worker`（Cargo フィーチャ `worker`）を使うと、`Worker` トレイトと `export_worker!` で次のように書けます。

```rust
use yoake_wasm_sdk::worker::{self, Worker};

#[derive(Default)]
struct MyWorker;

impl Worker for MyWorker {
    fn run(&mut self) {
        while let Some(message) = worker::recv() {
            // message を見て、通信やファイル処理をする
            let _ = worker::send(b"done");
        }
    }
}

yoake_wasm_sdk::export_worker!(MyWorker);
```

フルアクセス MOD（`world = "full-access"`）からも、`yoake_guest_sdk::worker::{send, recv}` で同じワーカーを使えます（[`crates/yoake-guest-sdk/examples/f1-f2-demo/`](../../crates/yoake-guest-sdk/examples/f1-f2-demo/src/lib.rs)）。

## 決まりと上限

- キューは行きと帰りに 1 つずつ。最大 **1024 件・16 MiB** です。
- いっぱいのときは、送るとエラー（`queue-full`）が返ります。待ちません。
- ワーカーが落ちたとき（トラップ・パニック・`exit`）、止まるのはそのワーカーだけです。ログに行が出て、MOD は動き続けます。以降の送信は `closed` のエラーになります。
- MOD をアンロードしたとき（ホットリロードなど）と、ゲームを終了したときに、ワーカーも止まります。

## 古い `[wasm] permissions` について

サンドボックスの MOD のサービス（設定・権限・保存）は、`permissions` に書かなくても使えるようになりました。古い MOD が読み込めるように、書いてあっても読み取ります。

サンプルは [`crates/yoake-wasm-sdk/examples/f2-io`](../../crates/yoake-wasm-sdk/examples/f2-io/src/lib.rs) と [`crates/yoake-wasm-sdk/examples/f2-check`](../../crates/yoake-wasm-sdk/examples/f2-check/server/src/lib.rs) にあります。くわしい仕様は [`docs/yoake2/MOD-GUIDE.ja.md`](../yoake2/MOD-GUIDE.ja.md#15-ファイルネットワークワーカー-fr-f2) の第 15 章です。
