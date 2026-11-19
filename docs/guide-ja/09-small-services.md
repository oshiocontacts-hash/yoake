# 9. 画面・インベントリ・音・食料のサービス

この章は、`client-full` と `server-full` の小さなサービスをまとめます。
どれも、使わなければ何も変わりません。くわしい仕様は `docs/yoake-wasm-dev.md` の「FR-S1」にあります。

---

## 9-1. 画面の入力フックから送信する

`net-send-to-server` は、画面のキーやマウスのフックからも呼べます。
`screen-key-pressed-pre`、`screen-char-typed-pre`、`screen-mouse-*`、`screen-click` も同じです。
1 ティックに送れるのは 32 通までです。

## 9-2. 回転したテクスチャーを描く

`hud-command::quad` に `sprite` を入れると、四角形にスプライトを貼れます。
`sprite` は GUI のスプライト名です。GUI テクスチャーなら `texture:<id>` と書きます。

1. `client::gui_texture_id(&texture)` で `id` を取ります。
2. `client::rotated_corners(x, y, w, h, 角度, 回転の中心)` で、回した 4 つの角を作ります。
3. `HudQuad { corners, sprite: Some(format!("texture:{id}")), uv: None, color }` を返します。

`hud-command::texture` も、同じ `id` で描けます。
`item-decorations` が返すコマンドでも使えます。

## 9-3. インベントリを読む

- `client::screen_carried()` は、画面を開いているときにカーソルにある物を返します。
- `client::local_player_stacks()` は、自分の持ち物を返します。ホットバー、メイン、防具、オフハンドの順です。空のスロットは入りません。

## 9-4. クライアントだけの音

- `client::play_local_sound("namespace:path", 音量, ピッチ)` は、位置のない UI の音です。
- `client::play_local_sound_at(位置, "namespace:path", 音量, ピッチ)` は、ワールドの中の音です。

音量は 0 から 16、ピッチは 0 から 8 です。音は次のフレームで鳴ります。

## 9-5. 食料を設定する

サーバー MOD は `server::ext::set_food(プレイヤー, 食料, 満腹度)` を呼べます。
食料は 0 から 20 です。満腹度は食料を超えません。

## 9-6. ブロックエンティティを書き換える

`block-entity-tick` などが返す `entity` は、エンジンが書き込みます。
クライアントに見せたいときは、`block-entity-sync` を呼びます。

## 9-7. キーを消費する

`key-pre` は、プレイ中のキーを、キーバインドより先に受け取ります。
`true` を返すと、そのキーは消えます。
押したキーを消したら、離したキーも消してください。Escape は渡されません。
