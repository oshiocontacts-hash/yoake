# ゲーム設定（起動前オプション設定） / Game settings (pre-launch options)

## 日本語（プレイヤー向けガイド）

ランチャーから、Minecraft を起動する直前に `options.txt` の設定をあらかじめ変更して起動できます。  
たとえば **「BGM 音量を 0% にする」「視野 (FOV) を 90° にする」** といった設定を一度作っておけば、新しいインスタンスを作ったときもそのまま適用されます。

### 2 つの設定場所

1. **いつもの設定（ランチャーの「設定」→「ゲーム」）**  
   すべてのインスタンスで共通して使われる既定プロファイルです（ランチャーのデータフォルダー内 `game-options.json` に保存されます）。
2. **インスタンスごとの「ゲーム設定」（ホーム画面の「ゲーム設定」タブ）**  
   そのインスタンスだけに適用する設定です。「いつもの設定を使う」がオン（初期値）のときは、まず「いつもの設定」を適用し、同じ項目がある場合はインスタンス側の設定で**キーごとに上書き**します。

### 適用モード：「毎回」と「最初だけ」

各項目には、行の右側にあるチップをクリックして切り替えられる 2 つのモードがあります。

- **毎回（Every launch）**: 起動するたびにランチャーの値で `options.txt` を上書きします。ゲーム内で一時的に変更しても、次回起動時にはランチャーの設定に戻ります。
- **最初だけ（First time only）**: インスタンスで最初に起動するときと、ランチャー側でその項目の値を変更したあとの次の起動時だけ `options.txt` に書き込みます。その後はゲーム内での変更がそのまま残ります。

### 設定できる項目

- **サウンド**: マスター音量、BGM (音楽)、ジュークボックスと音符ブロック、天候、ブロック、敵対的生物、友好的生物、プレイヤー、環境音、声・スピーチ、UI サウンド
- **ビデオ・描画**: 視野 (FOV 30–110°)、描画距離、演算距離、GUI スケール、明るさ、最大フレームレート、画面の揺れ
- **操作・言語**: マウス感度、ゲームの言語
- **その他の設定**: インスタンスの現在の `options.txt` からキーを検索し、任意の `key:value` を生の値で設定できます。

### 便利なボタン（インスタンスの「ゲーム設定」タブ）

- **今のゲーム設定を読み込む**: そのインスタンスの現在の `options.txt` を読み込み、ゲーム設定リストに反映します。
- **いつもの設定にコピー**: このインスタンスに設定している項目を、ランチャー共通の「いつもの設定」へコピーします。

> Horizon / Yoake インスタンスと Minecraft: Java Edition インスタンスのどちらでも同じ仕組みで起動前に反映されます。未知のキーやほかの行の順序はそのまま保たれます。

---

## English (Player Guide)

You can configure Minecraft's `options.txt` directly from the launcher before the game starts—for example, setting **Music (BGM) to 0%** and **FOV to 90°** once and reusing them across all your instances.

### Two levels of settings

1. **Usual settings (Launcher Settings → Game)**  
   Your global defaults, saved in `game-options.json` inside the launcher's data folder and shared across instances.
2. **Per-instance Game settings (Home → Game settings tab)**  
   Overrides for a single instance. With **Use usual settings** enabled (on by default), Usual settings are applied first and the instance's own entries override them **key by key**.

### Apply modes: Every launch vs First time only

Each row has a clickable mode chip:

- **Every launch**: Written to `options.txt` before every launch. Changes made in-game are overwritten on the next launch.
- **First time only**: Written on the first launch of an instance, and once more if you later change that option's value in the launcher. Subsequent in-game changes are preserved.

### Supported options

- **Sound**: Master volume, Music (BGM), Jukebox / Note blocks, Weather, Blocks, Hostile creatures, Friendly creatures, Players, Ambient / Environment, Voice / Speech, UI sounds
- **Video & Graphics**: FOV (30–110°), Render distance, Simulation distance, GUI scale, Brightness, Max FPS, View bobbing
- **Controls & Language**: Mouse sensitivity, Game language
- **Other options**: Search keys currently present in the instance's `options.txt` (or enter any key) and set raw `key:value` entries.

### Convenience actions (Instance → Game settings tab)

- **Load this instance's current settings**: Reads the instance's existing `options.txt` and fills the profile from it.
- **Copy to usual settings**: Copies the instance's configured entries into your global Usual settings.

> Works identically for both Horizon/Yoake instances and Minecraft: Java Edition instances. Unconfigured keys and existing line order in `options.txt` are preserved.
