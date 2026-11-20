# FR-F4M-4c: riding, interaction, taming and breeding for fully custom mobs

Branch `yoake-free/f4m-4c`, from `yoake-free/f4m-5b` (`54d50bb2f`). Not pushed.

## Step 0 (F4M-5b reviewer decisions)

`anim_sync::ANCHOR_TOLERANCE` (20 ms) and `remote::FRESH_TRIGGER_SECONDS` (0.25 s) are named, documented constants (the reasons are in their doc comments), and the test
`a_late_trigger_then_a_re_anchor_fires_its_time_zero_event_once` pins that a trigger heard of 0.1 s after its time-0 event, followed by a repeated trigger message and a new layer anchor, fires that event
exactly once. Noted in `FR-F4M-5b.md`.

## What landed

* **Schema** (`horizon-mods/src/custom_mob.rs`): `saddle_item`, `rideable_when` (a safe expression) and `interact` (rows of `feed`, `tame`, `ride`, `give_item`; the work item names the shortcuts, the schema had no place for `give_item`'s data, so the rows are new);
  `LoadedModel::locator_rest_position`. `tameable`, `breedable` and `seats` already existed. The old goal types are unchanged; `follow_owner`, `sit`, `owner_hurt_target`, `owner_hurt_by`, `owner_target`, `breed`, `follow_parent` now run (40 of the 41 goal types; only `script`, F4M-10, is deferred).
* **`mobs/custom/ai/tame.rs`**: `tameable` once, natively, on the engine's own owner state (`extra.owner`, `tame`, `sitting`; the vanilla `tame.rs` is not edited, only `owned_by` is reused). `try_tame` (chance on the mob's own random),
  `tame`/`untame`, `set_sitting`/`toggle_sit` (mirrored into the type's pose value: a `bool` `sitting`/`sit`, or the enum `state` with a `sit` value), goals `FollowOwner` (start 10, stop 2, teleport at 12), `Sit`, `OwnerHurt` (`owner_hurt_target`, `owner_hurt_by`,
  `owner_target` = either, the more recent). `tameable.follow_owner` / `sits_on_command` add the default goal when the rows lack it; a tameable type that attacks gets the owner target goals by default (priorities 1 and 2).
* **`mobs/custom/ai/breed.rs`**: `breedable` once: `feed` (love 600 ticks; a baby grows a tenth; a tameable type only when tamed), `Breed` goal (walks to a partner of `partner`'s type, 60 ticks, emits the engine's `MobEvent::Breed`), `FollowParent`, `tick` (baby growth and the pause countdown, which the engine only runs for vanilla species,
  and the baby's model scale), `complete` (partner pause, baby age -24000 and scale, tamed parent hands the owner on, `mob-bred` notice), `offspring_kind`.
* **`mobs/custom/ai/riding.rs`**: `RideDef` (seats resolved against the model; a locator seat sits at the locator's rest position), `assign`/`free_seat` (stateless: the player takes the first seat that allows it, mobs the next free allowed ones), `rideable` (saddle, owner of a tameable type, `rideable_when`),
  `takes_saddle`, `seat_position` (static offset turned with the yaw; takes `client_locator` as the **F4M-6 hook**), `attachment`, `mob_rider_attachment`, `controlled`, `locked_yaw`.
* **`mobs/custom/ai/interact.rs`**: the `mob-interact` plumbing. `Hook` trait (the F4M-10 WASM hook implements it), `set_hook`, `dispatch_global` (what the game calls), `dispatch(mob, ctx, hook)` (what tests call with a double). Order: hook (`Consume`/`Success` end it, `Pass` goes on), the `interact` rows, then the implicit shortcuts
  (saddle, tame, feed, owner's sit order, ride). The result is an `Outcome` with `Reply` (`Consume`, `Pass`, `Success`) and `Action`s only the game can do.
* **Notices** (`CustomState.notices`, `take_notices`): `Notice::Tamed { owner }`, `Notice::Bred { partner, child }`, queued like `events` (at most 8). F4M-10 drains them into `mob-tamed` / `mob-bred`.
* **Persistence** (`persist.rs`): `yoake.owner` (int array), `sitting`, `age` (a baby's negative age or the pause), `love`; written only when not default, dropped from an older save when they no longer hold.
* **`sync: owner`** (`lan/mob_sync.rs`, `game/lan_mob_sync.rs`): `Collected.owner_delta` / `owner`, `owner_mask`, `snapshot_for(mob, viewer)`, `for_viewer_as(viewer, ...)`. An owner value goes to the player whose uuid is the mob's owner, in its snapshot and in a delta of its own; with no owner it is sent to nobody and stays pending; taming marks every owner value pending.
  No wire change (the same `Delta`/`Snapshot` messages, a different audience).
* **Game glue** (`game/custom_mob_interact.rs`, new): `interact_custom_mob` (item tag matcher, the seat the player may take, the actions) and `custom_breed` (the baby of `offspring`, then `breed::complete`).

## Integration points in shared files (all vanilla neutral: custom mobs only)

| File | Edit |
|---|---|
| `game/creatures.rs` `interact_mob` | one `if kind.custom.is_some() && interact_custom_mob(index) { return Ok(true) }` before the vanilla handlers that follow the name tag, shears, spawn egg and lead (a `Pass` falls through as before). |
| `game/creatures.rs` `breed` | one `if p.kind.custom.is_some() { custom_breed; return }` after the shared part (the mod's baby event, the credit and the advancement, the hearts); `spawn_breeding_orb` is `pub(super)`. |
| `mobs/riders.rs` `base_attachment` | arm `Species::Custom => riding::attachment(v)`. |
| `mobs/equine.rs` `controlled` | arm `Species::Custom => riding::controlled(m)` (before, the `_` arm gave `extra.saddled`). |
| `mobs.rs` `Mob::tick` | `breed::tick(self)` inside the existing `if kind.custom.is_some()`. |
| `game/passengers.rs`, `game/mounts.rs` `position_rider` | a branch for a custom vehicle (seat of a mob rider; the player's seat, less the player's 0.6, and the yaw lock). |

No new `yoake_hook` ids; `Mob::tick` has the same signature.

## Tests

Before the merges (F4M-4c on f4m-5b): `-p horizon-client-lib --lib` **1801 passed** (1773 + 1 step-0 test + 27 new), `-p horizon-client --lib` 1203, `-p horizon-mods` 59 (+1), `-p horizon-protocol` 227.
After merging `yoake-free/merge` (fed161c18) and `yoake-free/f4m-robust` (7ce41f690): `horizon-client-lib --lib` **1825 passed**, 48 ignored; `horizon-client --lib` **1228 passed**, 50 ignored; `horizon-mods` **59**; `horizon-protocol` **227**; `horizon-mobmodel` all test binaries pass. 0 failures.
New tests: `ai/tests_4c.rs` 24 (tame chance by seed, berries tame and notice, sit order, sitting stays put, follow owner, far owner teleport, default goals, owner_hurt_by / owner_hurt_target / owner_target, exclusions, feeding, breeding and cooldown, baby clock and scale, offspring/partner types, tamed baby, follow_parent, passenger position by the server offset, seat assignment, saddle/owner/rideable_when, ride click, hook double and ordering, installed hook, rows and tags, Mob::tick clock), `persist` 1 (owner, sitting, age, love round trip), `lan::mob_sync` 1 (owner values to the owner only, snapshot and delta), `horizon-mods` 1 (schema), `remote` 1 (step 0).
Also: the derived default attribute set (`mod_content`) is now the union of the non-misc vanilla rows' keys with the attribute registry's default values (`attributes.json`); the existing test was rewritten for it (28 keys incl. `tempt_range`).

## Merges

`yoake-free/merge` and `yoake-free/f4m-robust` merged without conflicts (`mobs/custom/spawn.rs` was not touched here). Fix-up: `horizon-mods/src/custom_mob.rs` `load_model` passes the asset path as `file` to `parse_animation_file` and `parse_controllers` (their error text now carries it). No custom-mob hook ids were handed out. `regen.sh` was not run.

## Needs checking

* **Parity (Yoake):** not run (`f4m_parity` needs game resources). Nothing changes without a custom type: every new branch is behind `kind.custom.is_some()` or a `Species::Custom` arm; `mob_sync::collect` is only reached for custom mobs; the persisted keys are only written by `persist::write`, which only runs for custom mobs.
* **bindgen:** no struct annotated for bindgen changed that I could find: `CustomState`, `CustomType` (new fields `tame`, `breed`, `ride`; `notices`, `scaled_from`), `Collected` and `Delta`-side types are not annotated; `Mob::tick`'s signature is unchanged and its body gained one call. `crates/yoake-bindgen/regen.sh --check` calls `cargo` directly, so it was **not run** (the work order forbids calling cargo directly): please run it.
* **Game glue is only compile checked** (`game/custom_mob_interact.rs`, the edits in `game/creatures.rs`, `passengers.rs`, `mounts.rs`): no game resources here, so a real click, a real ride and a real breeding were not run. The pieces they call are tested.
* The game's `breed` shares its start with vanilla animals (the mod hook `native_baby_entity_spawn`, `breeding_credit`, `award_custom_stat`, `trigger_bred` with a clone of the parent as the "child" view), so the advancement and statistic of a custom breeding behave as for vanilla animals.
* The riding of a custom mob goes through the engine's horse path (`tick_mount_riding`, `equine::ridden_step`: `forward`, `strafe`, `yaw`, no jump since `jump_strength` vector is 0). A real ride was not tried.

## Open questions

1. **More than one mob rider.** The engine links one mob rider per vehicle (`extra.rider`, `riders::start_riding` refuses a second), so the seats of a type are filled by at most the player and one mob. Seats beyond that are in the layout and in `assign`, and a `StartRiding` path for further mobs is not built. Do we need it for 1.0?
2. **`sync: owner` and the host's own window.** The integrated host reads the mob's state directly, so it sees owner values of every mob, not only its own. Say if the host's own window should be limited to mobs it owns.
3. **`owner_target`** is defined as "what the owner attacked or what hurt it, the more recent" (the design doc names it only as a target selector). Confirm.
4. **Tame food heals.** Vanilla tamed wolves heal from food. The work item does not say; a tamed mob here only uses `tameable.items` to tame. Add healing?
5. **Baby scale and a type scale.** The baby is the mob's `model_scale` times `baby_scale` while it is a baby; the box (`dims`) follows `model_scale`. A type that sets the `scale` attribute might need the two combined in the attribute layer (not looked at: custom attributes are F4M-3's). The engine's `fudge_position_after_size_change` is not called when the baby grows up.
6. **`rideable_when` expression inputs** are the goal rows' (`health`, `is_baby`, `synced(...)`...), evaluated on the server at the click; a seat the client shows as free is never trusted.
7. **Tag matching** of `items`/`food`/`interact.item` is done by the game's matcher (`#tag` through the item tags, else exact id); the pure natives take a matcher closure, so tests and F4M-10 pass their own.
8. The notices are queued and not drained by anything until F4M-10; the queue keeps the latest 8.

## Reviewer follow-up

1. **Host's own window and `sync: owner`** (open question 2, decided: filter). The integrated host reads the server mob's `CustomState` directly (nothing is sent to its own window). Nothing in the game reads the values yet (F4M-6/F4M-10 will), so the filter is the read point they must use:
   `lan::mob_sync::host_value(mob, host_player_uuid, name)` / `value_visible_to(mob, slot, viewer)` give a value only when a remote client of that uuid would be sent it (`sync: owner` only of a tamed mob the host's player owns; the same rule as `for_viewer_as`). Test: `the_hosts_own_view_hides_owner_values_of_mobs_it_does_not_own` (also checks it equals the wire result per slot).
