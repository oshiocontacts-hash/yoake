# Yoake gameplay injection

`inject-mod` は署名済み `.yak` の Mixin をゲーム処理へ接続します。
1.2 ではパッケージ設定の読み取りと変更通知を追加しました。既存の 1.1 injection world と
1.0 通常 hook world は変更せず、そのまま読み込めます。

The `inject-mod` world connects signed `.yak` mixins to the committed gameplay
catalog through Wasmtime. The ordinary 1.0 hook worlds are unchanged. Injection is
for trusted mod authors: the existing package signature, identity, fingerprint,
ABI, and engine-version checks still apply. A catalog entry is an explicit engine
boundary, not permission to call arbitrary Rust functions or dereference memory.

## Package and registration

A mixin package declares full `path#signature_hash` targets and a component for
every side required by those targets:

```toml
[yoake]
version = ">=0.2, <0.3"

[side]
client = true
server = true
required_on_join = false

[wasm]
interface = "1.1"
world = "inject-mod"
client = "bin/wasm/mixin-demo.wasm"
server = "bin/wasm/mixin-demo.wasm"

[mixin]
targets = [
  "world::living_fall::calculate_fall_damage#9a9a629c2a289ab6ffb883cb0cd5891af74587992a6e2da94d8090723ede5b94",
  "game::chat::handle_chat_input#2d6b616020cec7006f8b8c6aeb2d8f9faf0caa7a8dbeb7f38b5a4d839b0d2e62",
]
```

This is an excerpt; normal package metadata is still required. Server-only mixin
packages are supported. A `both` target may run in either or both components
provided by the package. Each component registers every declared target; the
host installs only registrations applicable to that component's side. The host
rejects an empty applicable registration set, duplicate registrations, undeclared
sites, unsupported points, unknown paths, stale signature hashes, wrong sides,
and components that do not implement the injection WIT. Target lists contain
1-1024 unique entries. A 1.1 or 1.2 injection component uses the separate injection world,
not an ordinary 1.0 hooks world in the same component.

The signature hash is SHA-256 over
`yoake-inject-v1:(<comma-separated argument shapes>)-><result shape>`.
Fallible Rust functions use `result<success-shape>` in that hash; their handlers
observe only successful values. Native errors bypass RETURN handlers. Engine
version requirements are checked before loading; a matching engine range does
not excuse a changed target signature.

## Writing a handler

Enable `yoake-wasm-sdk` with `default-features = false` and features `std,mixin`.
The SDK provides plain Rust callbacks and a registration table:

```rust
use yoake_wasm_sdk::mixin::{Handler, Value};

fn halve_fall_damage(result: Value) -> Option<Value> {
    match result {
        Value::Signed(damage) => Some(Value::Signed(damage / 2)),
        _ => None,
    }
}

static HANDLERS: &[Handler] = &[Handler {
    site: "world::living_fall::calculate_fall_damage#9a9a629c2a289ab6ffb883cb0cd5891af74587992a6e2da94d8090723ede5b94",
    head: None,
    returned: Some(halve_fall_damage),
    overwrite: None,
}];

yoake_wasm_sdk::export_mixin!(HANDLERS);
```

HEAD takes `&[Value]` and returns `HeadOutcome::Proceed`,
`HeadOutcome::ReplaceArguments(Vec<Value>)`, or `HeadOutcome::Cancel(Value)`.
RETURN takes the current result and returns `None` to keep it or `Some(value)` to
replace it. Overwrite takes `&[Value]` and supplies the result instead of running
the native body. A table entry may implement several points. The generated
`registrations()` uses bit 1 for HEAD, 2 for RETURN, and 4 for overwrite.

See [`mixin-demo`](../crates/yoake-wasm-sdk/examples/mixin-demo/README.md) for a
standalone component, manifest, and build command. It halves fall damage, doubles
the **bonemeal** crop-growth increment, and logs submitted chat. It does not double
all crop random ticks. Package staging and official signing remain owner tasks.

## 設定の読み取りと変更通知 / Config (1.2)

設定を読む injection component は SDK の `std,mixin-config` feature と
`[wasm] interface = "1.2"`, `world = "inject-mod"` を指定します。
従来の `std,mixin` は引き続き 1.1 component を生成します。
1.0 ABI baseline と 1.1 の import/export は変更していません。

```toml
[wasm]
interface = "1.2"
world = "inject-mod"
server = "bin/wasm/mixin-config.wasm"
permissions = ["service.config"]

[config.count]
type = "int"
default = 2
min = 1
max = 4
side = "server"
```

これは抜粋です。通常のパッケージ情報、`[side]`、`[mixin].targets` も必要です。
`yoake_wasm_sdk::mixin::config_get("count")` は通常の `ConfigValue` を返します。
呼び出し元のパッケージに固定され、別パッケージの ID を指定する引数はありません。
権限 `service.config`、宣言済みキー、型・範囲・side の検証、同期設定の server 権威は
通常の server/client module の `config-get` と共通です。この API は読み取り専用です。

`export_mixin!(HANDLERS, config_changed = on_config_changed)` で
`fn on_config_changed(key: &str, value: ConfigValue)` を登録できます。
初期値は `config_get` で読み、変更通知でキャッシュを更新してください。
通知は次の injection handler の実行前、および tick/frame 時に配送されます。
配送対象は `config-changed` を export し、`service.config` が許可された module のみです。
同じ配送までに同じキーが複数回変わった場合は最新値を一度だけ通知します。
通常 module が自身の通知を消費しても injection component の通知は失われません。
通知も handler と同じ fuel・実行期限の制限を受けます。
通知が trap した場合は module を無効化し、以後の通知を再実行しません。
パッケージサービスを取得できない場合は警告してサービスなしでロードを続行します。
その場合の設定呼び出しは拒否されます。`config@1.2.0` の import は、設定対応の
inject interface と config events を宣言する injection component のみに許可されます。
通常の client/server world や 1.1 のみの injection component には許可されません。

Config notifications run only for modules exporting `config-changed` with an allowed
`service.config` capability. A notification trap disables the module, including its
site masks; it is not replayed on later ticks or frames. If package services cannot
be obtained, loading warns and continues without services; config calls are denied.
The `config@1.2.0` import is restricted to injection components declaring both the
config-capable inject interface and config events, not ordinary client/server worlds
or injection components declaring only 1.1.

実装例は [`mixin-config/src/lib.rs`](../crates/yoake-wasm-sdk/examples/mixin-config/src/lib.rs)。
その example フォルダーで `cargo build --offline -j 4 --release --target wasm32-wasip2` を実行します。
設定の初期値と変更通知を使い、sleep timer の RETURN handler が返す値を更新します。

## ユーティリティ向けの追加箇所 / Utility sites

次の 5 箇所はすべて **server 側・RETURN のみ** です。handler がなければ vanilla の結果を使います。
完全な ID は署名ハッシュを含めて `inject-sites.json` から指定してください。

| 箇所 | 引数 → 戻り値 | 結果の制約 |
| --- | --- | --- |
| `game::split_orb_values` | `u32 → list<u32>` | `spawn_orbs` の XP 分割値。各値は 1〜32767（保存時の Short の上限）、合計 XP は厳密に同一、個数は vanilla 以下。0 XP は空リストのみ。 |
| `world::food::exhaustion_amount` | `f32 → f32` | `add_exhaustion` に加える量。有限値を 0〜40 に clamp。NaN・無限大は拒否。 |
| `game::explosion::affected_blocks` | `list<block-pos> → list<block-pos>` | entity damage/knockback の後、ブロック除去・drop の前。元の位置リストの部分集合のみ。全件・空も可。追加位置や余分な重複は拒否。 |
| `world::level::tick::fire_burn_odds_at` | `(block-pos, s32) → s32` | 可燃ブロックへの隣接燃焼 roll の odds。0〜100。既存の `fire_ignite_odds_at`（空間への着火）とは別。 |
| `game::sleeping_players` | `list<string> → list<string>` | 対象プレイヤー ID の部分集合で分母だけを絞る。睡眠中・熟睡中の人数は変更しない。ID は UUID、UUID のない local は `local`。 |

RETURN handler は mod ID 順に実行されます。部分集合は直前の handler の結果を基準に検証し、
後続 mod が一度除かれた要素を復活させることはできません。違反した handler は無効化し、
直前の有効な部分集合を維持します。XP の合計・個数など値の制約は元の native 結果を基準にし、
違反時は handler を無効化して native 結果へ戻します。
list/subset 箇所の変換上限は 266240 nodes、深さ 32、16 MiB で、65536 個の位置を扱えます。
それでも引数・native 結果が大きすぎて変換できない呼び出しは vanilla のまま処理し、
handler ごとに一度だけ警告します。handler 自体は無効化しません。
XP は handler がなければ従来の分割・spawn・乱数消費順をそのまま使います。
爆発はブロック破壊だけを絞り、entity damage・knockback は変更しません。
爆発後の着火は、絞り込み前の元の位置リストをそのまま使います。これは vanilla と同じ
乱数の順序・消費数を維持するためです。破壊を除外した位置も着火処理の対象に残ります。
sleep は従来の参加条件、sleep timer、百分率の切り上げ計算を維持します。

XP replacements must preserve the vanilla total, use no more orbs, and keep each
orb value within 1–32767 so the saved signed Short does not lose XP. Subset RETURN
handlers run in mod-ID order and may only narrow the preceding handler's result;
they cannot restore removed entries. An invalid subset response disables that
handler and preserves the last valid subset. Other value rules retain the original
native baseline and native fallback. List/subset conversion supports at least
65,536 positions within 266,240 nodes, depth 32, and 16 MiB. Calls whose engine
arguments or native result still exceed those bounds keep vanilla behavior and
warn once per handler without disabling it.

Explosion filtering changes block destruction only, not entity damage or knockback.
Incendiary fire deliberately continues to use the original, unfiltered target list
to preserve vanilla RNG draw order and count. Positions excluded from destruction
therefore remain eligible for the subsequent fire pass.

## Values and scoped handles

`yoake-value` is the host representation: unit, bool, signed/unsigned numbers,
float, string, bytes, list, and record. Numeric replacements must fit the catalog's
exact width. `usize`/`isize` use their host-width range. Strings are owned across
the WIT boundary; native `&str` arguments can be replaced by owned guest text.
`block-pos` is a three-element signed-32 list; `vec3` and `vec3-f32` are three-element
float lists. Block-state IDs are unsigned-32 values with a live registry bound:
the two state arguments of `on_place`, and the state arguments of
`crop_random_tick`, `crop_bonemeal_increase`, and `drop_falling_block`, must be
less than the current world's state count. A missing registry bound rejects the
replacement. Other block, item, and entity identities at these sites are immutable
scoped handles, not replaceable integer IDs.

Guest replacements reject NaN and infinity by default, including inside vectors.
`yoake-value::policy::site_policy` declares argument and result rules; the generated
`inject-sites.json` includes those rules for every site. All 60 sites are audited
for numeric and identity domains. Native arguments and native results are never
subject to these new restrictions, preserving unmodified vanilla behavior.

| Replacement domain | Inclusive bounds |
| --- | --- |
| Explosion radius | 0 to 128 |
| Sleep timer | 0 to 110 |
| Crop growth speed | 0.01 to 1024 |
| Bonemeal increment | 0 to 25 |
| Fire ignition odds | 0 to 10,000 |
| Fire spread-to-flammable-block odds | 0 to 100 |
| Damage amount | 0 to 1,000,000 |
| Exhaustion / saturation modifier | 0 to 40 |
| Food nutrition | 0 to 20 |
| Positions / item velocity, per component | -30,000,000 to 30,000,000 / -1024 to 1024 |

Remaining numeric rules (crafting, experience, enchantment, fall and jump inputs,
attack strength/speed, pickup delay, and result widths) are explicit in the same
catalog. Out-of-range values are rejected, except the explicitly clamped finite
`exhaustion_amount` result described above; its negative zero is normalized to
positive zero. Consumer arithmetic
for sleep, bonemeal, fire odds and crop random-tick divisors also saturates where
unchecked arithmetic could overflow.

Receivers and opaque/reference arguments are records containing nonzero `scope`
and `id` fields and a `type` string. An entity argument marked by the engine is a
scoped `entity` handle with its unsigned entity ID in `snapshot`. An item stack
handle has an immutable `snapshot` record containing `item`, `count`, and `damage`;
components and extra data are not exposed. There are no raw addresses or handle
mutation/accessor APIs in this step. Handles are valid only during their call and
must never be cached for later use. HEAD replacement must echo every original
handle unchanged, including its scope, type, and snapshot. Forged, cross-call,
or edited handles are rejected. Receivers occupy argument zero.

WIT cannot directly express this recursive value model, so `Value::Sequence` and
`Value::Object` carry indexed `ValueTree` node tables. The host validates a combined
budget of 4096 nodes, depth 32, and 256 KiB of payload per scalar-site conversion.
Sites with list arguments or results use the larger bounded budget described above.
Invalid indices, cycles, repeated or unreachable nodes, duplicate record keys,
and shape mismatches are rejected. `record_field` helps read SDK record nodes.
The Wasmtime store limit also bounds allocations made by component bindings.

## Ordering, conflicts, and failure

Handlers sort by `(bare mod ID, side)`, not a formatted ID/side string; for example,
`foo` precedes `foo-extras`. HEAD handlers
see argument changes made by earlier HEAD handlers. Cancel skips remaining HEAD
handlers, overwrite, and the native body, but RETURN can still transform the
cancelled result. Otherwise at most one enabled overwrite may replace the body.
Two overwrite registrations for the same site and side are a load-time conflict;
client and server overwrites are independent. RETURN handlers run in mod-ID order
and see earlier RETURN replacements. Invalid candidate registrations are rejected
before the registry publishes them. Publication is serialized by the host; each
site swaps its handler snapshot under its own lock. There is no globally atomic,
quiescent multi-site update across a client and integrated-server thread.

A trap or invalid response disables **that site/side/injection point**, logs the
mod, target, and failure, and uses the native fallback. A HEAD or overwrite failure
runs the native body once with the original arguments. If the native body already
ran after successful HEAD argument changes, a RETURN failure keeps that exact
cached native result: it does **not** re-run the body and does not keep an earlier
RETURN replacement, except at subset sites, where the preceding valid subset is
retained so a failure cannot restore entries removed by another mod. If HEAD
cancelled or overwrite succeeded, a non-subset RETURN failure
runs the previously uncalled body once with the original arguments. Guest or
native side effects are not rolled back.

A real component trap can poison Wasmtime's component call state. The next
surviving callback recreates that component with the same identity and policies;
its aggregate accounting is preserved and recreation time is charged. Guest
persistent state resets on this recovery. A failed recreation disables that
instance. Reentrant/busy instances and exhausted aggregate budgets skip callbacks
instead of deadlocking or disabling an otherwise healthy injection point. The
same skip/native-fallback behavior applies when an engine argument or native
result exceeds the host conversion budget; this warns once per handler and does
not disable an innocent handler. An empty target selection on one module side is a quiet no-op, not a
failure of the package's other module. The ordinary non-injection hook failure
policy is unchanged.

The runtime maintains separate client/server masks and refreshes them when an
injection point or instance is disabled. Unentered threads are `Unspecified` and
never dispatch; the client main thread explicitly enters `Client`. With no handlers,
the gate is a relaxed atomic load plus a branch, before any thread-local access,
locks, argument construction, or allocation.

## Sensitive sites and capabilities

The sensitive-site list for this catalog is exactly
`game::chat::handle_chat_input`, `game::commands::execute::execute_line`, and
`game::commands::execute_command`. Registering any injection point at these sites
requires `[wasm] permissions = ["service.commands"]`; a client-side registration
also requires `"service.chat"`, an explicit capability for outgoing chat/command
text rewriting. Other sites retain the ordinary injection capability set.

A changed HEAD command argument passes the same shared `command_gate` as the
command service: at most 64 KiB, no control characters, and at most 16 levels of
nested commands after `execute ... run` or `return run`. Literal `run` arguments
(for example, `/say please run stop`) do not start nested commands. Client chat
uses the outgoing Unicode trim before checking slash commands, and both command
send paths check again while guest-scoped. Denials show the translated command
failure message. The case-insensitive denylist (including `minecraft:` names and
nested commands) is: `transfer`,
`fetchprofile`, `publish`, `unpublish`, `debug`, `jfr`, `perf`, `stop`, `save-off`,
`save-on`, `op`, `deop`, `whitelist`, `ban`, `ban-ip`, `pardon`, `pardon-ip`,
`setidletimeout`, `test`, `compute`, `pregen`, `schedule`, `clone`, `data`, `item`,
`reload`, and `datapack`. Guest-scoped commands also reject placing command
blocks or signs, block-entity NBT in `setblock`/`fill`, item components in `give`,
command-block minecarts or entity NBT in `summon`, and NBT destinations in
`execute store`. The broad editing and reload restrictions conservatively block
deferred command authoring, including function load-tag reloads. These checks do
not filter the player's unchanged native text, and capabilities cannot bypass
them.

Guest provenance follows commands across the integrated server's in-memory
connection, including signed commands and quiet batches, and is stored with
queued commands. The server restores the scope before its command, function,
leaf and native-hook checks. The internal envelope is rejected on TCP/LAN guest
connections; a dedicated server is not trusted to honor a client flag. Existing
chat signature and identity validation still applies to the enclosed packet.

This scope is not persistent taint tracking. Deferred world work caused by an
allowed command later runs at that work's own permission level. In particular,
`run_command` click events authored by guest-scoped `tellraw` or `dialog` are not
stripped: a later player click runs unscoped, with the player's permissions.
Crafting count overrides must fit the live output item's maximum stack size;
unchanged vanilla recipe counts retain their native behavior.

## Sandbox limits

Injection uses the same store, fuel, epoch, and aggregate-budget policies as the
ordinary host hooks, with bounded value conversion in addition:

| Limit | Client | Server |
| --- | --- | --- |
| Per-call deadline | 20 ms | 25 ms |
| Aggregate execution per 50 ms window | 10 ms | 25 ms |
| Consecutive exhausted-window strikes before disabling | 20 | 20 |
| Fuel per call | 10,000,000 | 10,000,000 |
| Linear memory | 64 MiB | 64 MiB |
| Memories / tables / instances | 1 / 8 / 16 | 1 / 8 / 16 |
| Elements per table | 65,536 | 65,536 |

Epochs advance every millisecond. Initialization has a 250 ms call deadline and a
1 s aggregate budget. WASI output is bounded to 64 KiB, 1024 characters per log line,
and 256 emitted lines per drain. No limits, package trust checks, or ABI checks
are relaxed for mixins.

## Committed sites and signatures

The original 51 sites below allow HEAD, RETURN, and overwrite; the five utility
sites added in MX2 allow RETURN only. Argument order includes
receivers. Use the **full ID**, including the hash, in the manifest and SDK table.
Signatures are canonical catalog shapes, not a promise to expose opaque Rust
objects. In particular, coarse entity ticking/death, animal targetability,
projectile/item spawning, and the movement-prediction exception check are not
universal interception of every entity, AI decision, spawn, or movement validator.

| Full target ID | Side | Arguments -> result |
| --- | --- | --- |
| `crafting::crafting_result_count#5e147b2cfbddd26373813d8c72c8f539918d9fd10de46b9fd3af0f2859f3376e` | both | `(scoped<&ItemStack>) -> u32` |
| `furnace::smelting_result_count#5e147b2cfbddd26373813d8c72c8f539918d9fd10de46b9fd3af0f2859f3376e` | server | `(scoped<&ItemStack>) -> u32` |
| `game::apply_use_cooldown#82aa94f717b88f2311b42287713ef26f00afdf2fc95b3118e269930f05420206` | server | `(scoped<&mutSelf>, scoped<&ItemStack>) -> unit` |
| `game::attack_strength_scale#e28527bc3c45193c7ad0406144863440989c9d229dac40a38e43a7c54768e6d3` | client | `(scoped<&Self>) -> f32` |
| `game::block_entity_ticks::tick_furnace#cdf90a437f18a89365314441a8da1739860701f3c9bc528b13cf25fedd9adea2` | server | `(scoped<&mutSelf>, block-pos) -> unit` |
| `game::break_block#cdf90a437f18a89365314441a8da1739860701f3c9bc528b13cf25fedd9adea2` | server | `(scoped<&mutSelf>, block-pos) -> unit` |
| `game::chat::advance_clock#003edb56c3779aef98f8fdbb3391a4f1c6649d4bff5f035d49ca0c9e7ce4fa39` | server | `(scoped<&mutSelf>) -> unit` |
| `game::chat::handle_chat_input#2d6b616020cec7006f8b8c6aeb2d8f9faf0caa7a8dbeb7f38b5a4d839b0d2e62` | both | `(scoped<&mutSelf>, string) -> unit` |
| `game::commands::change_experience#115e5886164880616124c770066d1583d2372e9482f036a772a47d4907269211` | server | `(scoped<&mutSelf>, bool, bool, s32) -> bool` |
| `game::commands::execute::execute_line#fbb37fd1e372d65fd16a3cd73dadf4628d887a07d2b74c0abc345c462f0fb97a` | both | `(scoped<&mutSelf>, string, scoped<Source>) -> s32` |
| `game::commands::execute_command#e624502589081ee29de772d224cb46e0d9b3c622034c8c853db1db442e849932` | both | `(scoped<&mutSelf>, string, scoped<&Context>) -> s32` |
| `game::commands::kill_target_in_level#837c1e63b0dc126c0968314fdd075bb771840eb47350120e7eddfd10d5389202` | server | `(scoped<&mutSelf>, scoped<Target>) -> unit` |
| `game::commands::unpack_opened_loot#d3170787041df8d00db972ad571de5abd58a0ecb053a90f8160561dda020cc96` | server | `(scoped<&mutSelf>, block-pos, string) -> unit` |
| `game::consume_held#003edb56c3779aef98f8fdbb3391a4f1c6649d4bff5f035d49ca0c9e7ce4fa39` | server | `(scoped<&mutSelf>) -> unit` |
| `game::creatures::add_projectile#dd3b1deb840293eedb1d5770e2b1a690933d75ca3078fb142ee56c83a0760161` | server | `(scoped<&mutSelf>, scoped<horizon_client::projectiles::Projectile>) -> unit` |
| `game::creatures::drop_death_loot#2e402a9829ae42a363e722af51fa9b55ac490138f33e93fcafad14684c1e1c11` | server | `(scoped<&mutSelf>, scoped<&mutMob>, scoped<Option<u128>>, bool, bool, vec3) -> unit` |
| `game::creatures::hurt_player_by#95190a49eb1a130a876f949d602e3d5ebf39b11a69eebe07957684e83d9fb1ad` | server | `(scoped<&mutSelf>, string, f32) -> unit` |
| `game::creatures::mob_died_with_credit#4e21a94cb5744962c2314e952f3b8a6fa6f5c0b467c10250cbbe51e8bcdee1a2` | server | `(scoped<&mutSelf>, scoped<&Mob>, scoped<implFnOnce(&mutSelf,&Mob,Option<Option<u32>>)>) -> unit` |
| `game::creatures::mob_hits_mob#da775c291c1dfc2d03bf532d5e879e09e904fe8b4fa91292a26f6ef74648461f` | server | `(scoped<&mutSelf>, scoped<entity>, f32, scoped<Damage>) -> bool` |
| `game::creatures::mob_hits_player#976890241e05c11059763762a6101de1f2644a81857a0ed671c267f41940454e` | server | `(scoped<&mutSelf>, f32, vec3, string) -> bool` |
| `game::creatures::projectile_hit#5db96b45a2c79caf2e477618823ca0de828f3147d3fd57802926c2ee3122cca3` | server | `(scoped<&mutSelf>, scoped<&Projectile>, scoped<Hit>) -> unit` |
| `game::creatures::tick_creature_entities#3103cc0406cfb6da3480dc27c8a3e4a5472f9eedbfee20dc6f0ecdd59fabfd08` | server | `(scoped<&mutSelf>) -> unit` |
| `game::drop_falling_block#6887e3e4a647a52a1be65306a14d144a3eaad5307e9827caaa00336636d1103f` | server | `(scoped<&mutSelf>, block-state-id, vec3) -> unit` |
| `game::explosion::server_explode#910e0323d56abb8543f5f43fc61744ff4eb8b9e7404ab49e586a697a256c80d3` | server | `(scoped<&mutSelf>, vec3, f32, scoped<BlockInteraction>, bool) -> unit` |
| `game::hand::attack_damage#798a72923c4015b370175cb5ef3c9caafdccea63d164e658ba38a4a5bfbed4b3` | client | `(scoped<&mutSelf>) -> f32` |
| `game::hand::main_hand_attack_speed#53d3641715fcbb687a0abc0374630960332094b9d0c6ee58f3060ed023581853` | client | `(scoped<&Self>) -> f64` |
| `game::hand::reset_attack_strength#003edb56c3779aef98f8fdbb3391a4f1c6649d4bff5f035d49ca0c9e7ce4fa39` | client | `(scoped<&mutSelf>) -> unit` |
| `game::hand::use_with_hand#3103cc0406cfb6da3480dc27c8a3e4a5472f9eedbfee20dc6f0ecdd59fabfd08` | client | `(scoped<&mutSelf>) -> unit` |
| `game::machines::crafter::crafter_eject#cf1e3217c8403f4f25b9106dd358087db8fff262703ed7cf7d9672a84f0f1637` | server | `(scoped<&mutSelf>, block-pos, scoped<Direction>, scoped<ItemStack>, string) -> unit` |
| `game::native_hooks::knockback#82a1f96545afa67c62defbef8b195411c9c119ec73bd76e2cfcad0f71f21fbca` | server | `(scoped<Hurting<'_>>, scoped<&mutKnock>) -> bool` |
| `game::place_used_fire#cdf90a437f18a89365314441a8da1739860701f3c9bc528b13cf25fedd9adea2` | server | `(scoped<&mutSelf>, block-pos) -> unit` |
| `game::spawn_item#930a3e860bdfc085de2ed157bf46814b45129e44eab20760767c5e39e2e62112` | server | `(scoped<&mutSelf>, scoped<ItemStack>, vec3, vec3, u32) -> unit` |
| `game::spawn_orbs#7f2480e65cbba956ee9b12f8c3a97d8da96c3b5297c0d977cfcd65bd8b126928` | server | `(scoped<&mutSelf>, vec3, u32) -> unit` |
| `game::tick_sleep_timer#198a5917f750358ff70ce3f80ede1aa2c142d914cbe7bed985f5b6f4ed29a7b7` | server | `(u32, bool) -> u32` |
| `game::tick_weather_effects#003edb56c3779aef98f8fdbb3391a4f1c6649d4bff5f035d49ca0c9e7ce4fa39` | server | `(scoped<&mutSelf>) -> unit` |
| `game::try_sleep#f16a185525735bc55ac1bcaf5d932a202f5f4db381ac0d4a11457845dadb322a` | server | `(scoped<&mutSelf>, block-pos) -> unit` |
| `items::enchant::enchantment_cost#1b4f10a33cd7d201c2bdf989710857c0a8071491ca5adb526eb5f4013e196253` | server | `(scoped<&Self>, scoped<&mutR>, usize, s32, string) -> s32` |
| `lan::prediction::excused#895abb5ec4b04f60dad553e5912c9adc4ed4fb6551f2779c28670ea2b6fb7ee8` | server | `(scoped<&Self>) -> bool` |
| `mobs::animal_brain::targetable#6a280e20f4f62541857d28da6fdcebf69483ea0cfbb9670d696ffca8efc7ada1` | server | `(scoped<&mutMob>, scoped<&MobWorld>, scoped<Who>) -> bool` |
| `motion::set_sprinting#3f7fc74e6c415236ce9e86a6a011ef3bf5ec6893fde7f8861375c0c28c607f0d` | both | `(scoped<&mutSelf>, bool) -> unit` |
| `world::food::add_exhaustion#ca3064943a7660df0f8bb96751c0c58f9df0074453c066cff57d287963b4ef4e` | server | `(scoped<&mutSelf>, f32) -> unit` |
| `world::food::eat_properties#87c58205030f0859d15bbfbb4c03bca96f245feb674e1573b06406c03cc790eb` | server | `(scoped<&mutSelf>, s32, f32) -> unit` |
| `world::level::bonemeal::crop_bonemeal_increase#b61d671fb9375f084f2bf026f466add0e8423efd996180dee3fb52ca18348fd1` | server | `(scoped<&mutSelf>, block-state-id) -> s32` |
| `world::level::on_place#f2e7f4f622cd6459cdf4408595ab0732a6f751918833cd15a2c7cf691c14d14b` | server | `(scoped<&mutSelf>, block-state-id, block-pos, block-state-id, bool) -> unit` |
| `world::level::tick::crop_growth_speed#86f1053f51751d3d36e310304589b477148dcb9afff1a901c25bc8c3b283ca29` | server | `(scoped<&Self>, scoped<super::states::BlockId>, block-pos) -> f32` |
| `world::level::tick::crop_random_tick#28c6b43b0b3b1d1bec9292bba55e4f503787ebd89f597ed89bc0f87991833d61` | server | `(scoped<&mutSelf>, block-state-id, block-pos) -> unit` |
| `world::level::tick::fire_ignite_odds_at#e4b19b03408d40ebc13322180f28da0353f8d89dce91d334aec80d1a6bd951c7` | server | `(scoped<&Self>, block-pos) -> s32` |
| `world::living_fall::calculate_fall_damage#9a9a629c2a289ab6ffb883cb0cd5891af74587992a6e2da94d8090723ede5b94` | server | `(scoped<&mutW>, f64, f32) -> s32` |
| `world::living_fall::cause_living_fall_damage#40abf719eadc611c3b8f1173d44e1a693ec8bf8272f230f5f2af30b66ef64eaa` | server | `(scoped<&mutW>, f64, f32) -> bool` |
| `world::living_motion::jump_from_ground#49e2162006d6305be832360344cecf87f36422ed6eca7fe24c598d1fd965dc2a` | server | `(scoped<&mutW>) -> unit` |
| `world::living_motion::jump_power#a887a93bbf10f3908aa90913b32c0a6c6f5307dccc4a8d25ae9a4ac456432c1f` | server | `(scoped<&mutW>, f32) -> f32` |
| `game::split_orb_values#6ba2da0be9c6b32ac47dee8ac3f1249feba2b33441b2319cd0c9a5127041e9ef` | server | `(u32) -> list<u32>` |
| `world::food::exhaustion_amount#1afa78d578bf691d7976b467eb462df1a7c43752cfb449169c767bab158831bd` | server | `(f32) -> f32` |
| `game::explosion::affected_blocks#48eb22f452491e6764e4413bdb03f836666a322dba624fc9005ba4b67ed0b6d8` | server | `(list<block-pos>) -> list<block-pos>` |
| `world::level::tick::fire_burn_odds_at#5df3d466687228c4989c85fa0e6de7efa8071d5811a8aa63b47a4d4403274f68` | server | `(block-pos, s32) -> s32` |
| `game::sleeping_players#198b29101e378ffe727e9973838481bdce6987266d2f80cedb6979184cfe36f3` | server | `(list<string>) -> list<string>` |

## Adding an engine boundary

Use `#[yoake_inject(id = "stable::path", side = "server")]` on a real function or
method; choose `client`, `server`, or `both` explicitly and optionally restrict
`allow = "head,return"`. The owning crate depends on `yoake-inject-macro` and
`yoake-inject-rt`. Methods, generic bodies, borrowed/opaque arguments, native
`Result`/`?`, and early returns are supported. Async, const, unsafe, extern,
variadic functions, trait declarations, and conditional site definitions are
rejected. Mark entity-ID parameters with `entities = "parameter_name"`; otherwise
a plain unsigned integer is just a number. Add reviewed snapshot implementations
rather than exposing engine references to a guest.

The generator scans Rust syntax in the world/items crates, gameplay sources,
and the explicitly selected crafting, furnace, movement, prediction, and animal
brain modules. It emits `inject-sites.json`, the runtime's static site table, and
the injection WIT. It is not a general compiled-module-graph discovery mechanism.
Regenerate with `YOAKE_UPDATE_WIT=1 cargo test --offline -j 4 -p horizon-mod-api-gen --lib`
and review the committed changes. Normal generator tests fail if the catalog,
runtime table, or WIT is stale. A new boundary still requires an engine update;
a mod can choose among existing cataloged boundaries without one.

## No-injection cost and measurement

An annotated function first performs a single relaxed mask load and branch.
With all masks clear, it enters the original body before making snapshots,
allocating, taking registry locks, or consulting side thread-local state. Tests
check the no-allocation path and exercise that path while its handler lock is
held. They do not substitute for inspecting optimized builds on each platform.

The ignored `zero_mask_microbenchmark` compares otherwise equivalent no-inline
plain and annotated scalar functions. On the Windows test environment, a release
run (11 alternating samples of 20 million calls) measured medians of 1.5283 ns
plain and 2.1477 ns annotated: **0.6194 ns extra per call**, or 40.53% for this
intentionally tiny body. The `realistic_zero_mask_microbenchmark` compares one
million calls per sample of the actual annotated `calculate_fall_damage` against
an unannotated copy of its arithmetic and immunity check. Its 11 alternating
samples measured medians of 8.0660 ns plain and 8.8594 ns annotated:
**0.7934 ns extra per call**, or 9.84%.

These microbenchmarks do not meet the 1% target. Repeated runs on this shared
Windows environment vary substantially; neither these measurements nor a faster
sample establish whole-engine overhead below 1%. The zero-mask gate avoids TLS
and dispatch before the native body, but its load and branch still have a
measurable relative cost for short functions. Real engine-frame benchmarks and
vanilla parity runs still need an owner machine with prepared data; this
development run does not start the game or prepare Mojang resources.

Run the microbenchmarks explicitly with:

```sh
cargo test --offline -j 4 --release -p yoake-inject-rt --test zero_mask zero_mask_microbenchmark -- --ignored --nocapture
cargo test --offline -j 4 --release -p horizon-world --test inject_overhead realistic_zero_mask_microbenchmark -- --ignored --nocapture
```

## Follow-up / needs confirmation

- Whole-engine no-mod overhead below 1% and live vanilla parity remain unmeasured.
- Universal interception, compiled-module-graph discovery, and migration of the
  separate state-accessor work to `yoake-value` are outside this step.
- Scoped handles are read-only call snapshots, not a general state-accessor API.
- Multi-site hot publication is per-site consistent, not a global tick transaction.
- Official package signing, installation, and multiplayer policy require owner
  review and testing; no official signed package is produced here.
