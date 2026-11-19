# 9. アイテム・ブロックスプライトを動的に差し替える

`client-full` ワールドのクライアント MOD では、実行中にアイテムアイコンやブロックモデルのテクスチャ（RGBA8 画像）を部分書き換えして、アトラス全体を作り直さずに見た目を変更できます。

> [!NOTE]
> この機能はクライアント側の表示（HUD・手持ち・ドロップアイテム・ワールド内のブロック描画）だけを差し替えます。他のプレイヤーの画面には影響しません。

---

## 9-1. 使えるホストサービス

`yoake_wasm_sdk::full::client::services`（WIT インターフェース `client-services`）に以下の 6 つが用意されています。

| サービス | 引数 | 役割 |
|---|---|---|
| `item_sprite_set` | `(item_id, rgba, (width, height))` | 指定アイテムの共有アトラス上の矩形だけを `rgba` で上書きします（元の画像は内部に保存されます）。 |
| `item_sprite_reset` | `(item_id)` | `item_sprite_set` で上書きしたアイテムの見た目を、現在のリソースパックの元のテクスチャへ戻します。 |
| `block_sprite_set` | `(rgba, (width, height), texture_id)` | `mymod:block/pan` や `minecraft:block/stone` のようなブロックモデルテクスチャ層を `rgba` で上書きし、全ミップマップレベルを再生成します。 |
| `block_sprite_reset` | `(texture_id)` | 上書きしたブロックテクスチャ層を元のリソースパック画像（アニメーション含む）へ戻します。 |
| `item_sprite_pixels` | `(item_id)` | アイテムの現在のスプライトを `(width, height, rgba)` で返します（`item_sprite_set` の上書き後はその内容。アニメーションは先頭フレーム）。 |
| `block_sprite_pixels` | `(texture_id)` | ブロックモデルテクスチャの現在の画像を `(width, height, rgba)` で返します（上書き中はその内容、アニメーションは先頭フレーム。大きさはテクスチャ配列の 1 層の大きさ）。 |

- **`rgba` の形式**: 1 ピクセル 4 バイトの **RGBA8**（`[r, g, b, a, ...]`、上端行から左→右）で、長さは必ず `width * height * 4` バイトである必要があります。
- **サイズ不一致の扱い**: 対象スプライト／ブロックテクスチャ配列の実際の解像度と一致しない `size`（または 0 やバイト数不一致）はエラーを返し、アトラスを変更しません（自動リサイズは行いません）。
- **名前空間ルール**: 自分の MOD のリソースだけでなく、バニラや他 MOD のアイテム／ブロックテクスチャも上書きできます。別の MOD が後から同じ ID を上書きした場合は後勝ちになり、MOD のアンロード／リロード時は「その MOD が現在所有している上書き」だけが自動的に `reset` されます。
- **リソースパック再読み込み（F3+T 等）**: 上書き内容は保持され、再読み込み後の新パックに自動的に再適用されます。新パックでテクスチャ解像度が変わっていた場合などは、その上書きを警告ログを出してスキップし、クラッシュさせません。

---

## 9-2. Rust コード例（`client-full`）

```rust
use yoake_wasm_sdk::full::bindings::yoake::mods::client_services;

fn apply_tint_icon() -> Result<(), String> {
    // 16x16 RGBA8 の単色テクスチャ（16 * 16 * 4 = 1024 bytes）
    let mut rgba = Vec::with_capacity(16 * 16 * 4);
    for _ in 0..(16 * 16) {
        rgba.extend_from_slice(&[255, 80, 40, 255]);
    }
    client_services::item_sprite_set("minecraft:diamond", &rgba, (16, 16))?;
    client_services::block_sprite_set(&rgba, (16, 16), "minecraft:block/stone")?;
    Ok(())
}

fn restore_sprites() -> Result<(), String> {
    client_services::item_sprite_reset("minecraft:diamond")?;
    client_services::block_sprite_reset("minecraft:block/stone")?;
    Ok(())
}
```

---

## 9-3. 注意点

- **アニメーション付きブロックテクスチャ**: `block_sprite_set` 中は該当レイヤーのティックアニメーション更新が一時停止され、`block_sprite_reset` すると元のフレームとアニメーションが再開します。
- **スタック単位（NBT／カスタムデータ別）の差し替え**: 実行時のスタックごとの動的テクスチャ差し替え（`item-stack-sprite`）は共有アトラスへの動的スロット追加が必要になるため未実装です。スタックの見た目を変えたい場合は、バニラの `minecraft:custom_model_data` / `minecraft:custom_data` を使うアイテムモデル定義（`assets/<ns>/items/*.json`）を利用してください。

---

## 9-4. 現在のピクセルを読む（`*_sprite_pixels`）

- 読み出しはプレイヤーの手元で準備済みのリソース（アトラスの CPU 側コピー）から行うため、GPU からの読み戻しは起きず、MOD が呼ばない間は追加のメモリも処理もありません。アイテムごとに 1 回呼んでキャッシュする使い方を想定しています。
- 戻り値は `item_sprite_set` / `block_sprite_set` にそのまま渡せる並び（`width * height * 4` バイトの RGBA8）です。ブロックの `width` と `height` は、リソースパックの元の解像度ではなく、ブロックテクスチャ配列の 1 層の大きさです。
- 見つからない ID はエラーを返します。
