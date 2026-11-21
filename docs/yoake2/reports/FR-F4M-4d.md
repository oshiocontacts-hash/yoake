# FR-F4M-4d: several mob passengers, `seat-set`, `cooldown_persists`

Branch `yoake-free/f4m-4d`, from `yoake-free/f4m-fix` (`4e9b41cae`). Not pushed. Design: `FR-F4-MOBS.md` section 5, F4M-4c open question 1, F4M-10 deferred `seat-set`, F4M-FIX1 open question 2.

## 1. Reviewer decision 2: `cooldown_persists`

* Schema (`horizon-mods/src/custom_mob.rs`): top-level `cooldown_persists: bool`, default false (`#[serde(default)]`, a non-boolean is a file error).
* Default (absent or false): vanilla. A restarted `melee_attack` / `area_attack` clears its interval (`ChaseStrike::start` sets `until = 0`), exactly as before.
* `true`: `ChaseStrike` keeps the next-strike time in `CustomState.attack.strike_ready` (game time, not saved), sets it to `now + interval` when a strike begins, and `start` no longer clears the goal's timer. The cooldown runs out in game time while the goal is stopped.
* Scope, deliberately: `melee_attack` and `area_attack` (the goals whose timer starts at 0 on every restart and so strike at once). `leap_attack` already keeps its `cooldown` across restarts (`ready_at` lives in the goal, which lives as long as the mob), `ranged_attack` restarts with `attack_time = -1` and so waits a distance-based time on every start (vanilla `RangedAttackGoal`); neither is changed.
* Note for the F4M-FIX1 e2e: the goal also restarts by itself whenever a chase is over (`MeleeAttack::can_continue` ends when the navigation is done), so a mob standing next to its target restarts about every 20 ticks and, by default, strikes at every restart. That is what question 2 of FIX1 saw.

## 2. Reviewer decision 1: several mob passengers

### Where the engine stood (checked, not guessed)

* The **truth of who rides what is already a list**: each rider holds `extra.mount = Some(vehicle)`; `extra.rider` of the vehicle is only derived from it every tick (`Game::sync_riders`: the last rider of the list wins). `start_riding` was the gate that refused a second rider (`v.extra.rider.is_some() || v.rider.is_some()`), and `riders::link` and `sync_riders` kept one.
* **Persistence** already supports lists: `Passengers` is a list of entity tags; `take_mob_passengers` / `load_mob` load every entry as a rider of the vehicle and `put_mob_passengers` writes every mob with `mount == id`.
* **Network** already supports lists: `ClientboundSetPassengers` is `vehicle VarInt, passengers List<VarInt>` (`horizon-protocol` `play.rs`), `lan::entity::Riding` is `FxHashMap<i32, Vec<i32>>` built by `Game::lan_riding` from every mob's `mount`, and the client's `remote_set_passengers` sets `mount` for every listed mob and `extra.rider` to the first. A stock client handles a list as vanilla does.
* So nothing had to change in the encoder or the save format for the riders themselves. What was missing: the gate, the seats, and which seat each rider has.

### Design

* **Only custom vehicles with seats get several riders.** `riding::has_seats(m)` = a custom mob whose type has at least one seat. Every other vehicle (all vanilla ones, boats, minecarts, TNT, and a custom type without seats) takes the exact old code path.
* **Seat claims** (`CustomState.seat_claims: Vec<SeatClaim { rider, seat }>`, sorted by seat): which seat each mob rider holds. A seat is sticky: when an earlier rider gets off, the others keep their seats (4c derived the seat from the rider's place in the list, which would move riders). The player's seat is `CustomState.player_seat` (what the boarding click picked), else the first seat no mob holds that allows the player.
* `riding::board(mobs, rider, vehicle, seat, player_aboard, honour_cooldown) -> Result<usize, Refusal>`: puts a rider on the seat asked for (must exist, be free, and allow the rider) or the first free allowed seat; refusals are `Missing, NoSuchSeat, NoSeats, Itself, Loop, Cooldown, NotAllowed, Taken, Full`. The player and the mobs already aboard keep their seats. A rider already aboard moves.
* `riding::settle(mobs, vehicle_index)` (run for every custom mob from `riders::sync_links` once a tick): riders that came without a claim (an old save, `/ride`, a rider linked by `riders::link`) get one, riders no seat is left for (or that no seat allows, after a type change) are put off, claims of what no longer rides it are dropped, `extra.rider` = the mob on the lowest seat (so "ridden" flags keep working).
* **Callers:** `Game::try_mount_mob` (`/ride`, forced, no cooldown) and `riders::start_riding` (the AI's boarding, honours the 60-tick cooldown) use `board` for a seated custom vehicle and the old paths for everything else. `Game::dismount_mob` settles the vehicle afterwards. `Game::sync_riders` now calls the pure `riders::sync_links(mobs, other_vehicle_ids)` (the old loop moved as is; it is the vanilla code, see the argument below).
* **Positions:** the server places a mob rider with the seat's `offset` (`riding::mob_rider_attachment`, now by claim), `riders::seat` turns it with the vehicle. The client uses the same function, so server and client agree; the locator of the animated pose (F4M-6) is not used for mob riders yet (see open questions). A seat with `yaw_lock` also turns a *mob* rider with the vehicle (`passengers.rs`), as it did the player.
* **Driving seat:** the player steers from a `controls: player` seat only (4c, unchanged; `player_seat` now skips seats mobs hold, so a mob in seat 0 leaves the player the next seat, which may not steer). A mob rider steers the vehicle toward its own target (`riders::drive`) only from a `controls: player` seat (`riding::mob_may_steer`); a mob on a `none` seat is carried. **This changes 4c for one case**: a lone mob rider on a `none` seat used to steer; now it is carried (the seat description says `none` only carries).
* **Dismount on death/despawn:** `sync_links` is the old validity check: a rider or a vehicle that is dead (`health <= 0`) or removed lets go of everything; the other riders of a vehicle whose one rider died keep their seats. Tested for a dying rider, a dying vehicle and a removed vehicle.

### Vanilla preservation (why every old path is byte-for-byte the same)

* `riders::sync_links` is the former loop of `Game::sync_riders` moved to a pure function, with one added condition: a rider whose vehicle `has_seats` is not put in the `riding` map (and so is not given by `extra.rider = riding.get`); then `settle` runs for each custom mob. `has_seats` is false for every vanilla mob, boat, cart and TNT id. `settle` returns at once for a vehicle without seats and is only called for mobs with `kind.custom`.
* `riders::start_riding` has one new early branch, taken only when the vehicle has seats; the hoglin/piglin stacking, the loop check, the cooldown and the one-rider check below it are untouched.
* `Game::try_mount_mob` takes the new branch only for a vehicle with seats; `dismount_mob`'s added `settle` is a no-op for anything else.
* No field was added to `Mob` or `Extra` (the bindgen-annotated structs): the seat data lives in `CustomState`, which is not exported. No save key is added to a vanilla entity; no packet is sent for a vanilla vehicle.
* Tests that pin it: `a_vanilla_vehicle_keeps_its_one_rider_rules` (pig: second AI boarding refused; two forced riders keep the last as `extra.rider`; the vehicle's death lets both go; a rider on a boat id stays while the id exists; no custom state is created), `a_custom_type_without_seats_keeps_the_old_link_and_the_ai_boards_seats`, and every existing riding test (4c, jockeys, camel, hoglin) stays green.

### Persistence

* The riders themselves: the engine's `Passengers` list (it already was a list; nothing changed there).
* Their seats: the vehicle's `yoake` compound gets `passenger_seats: [int]`, one per saved passenger in `Passengers` order (-1: no seat), written by `custom::persist::write_passenger_seats` from `Game::put_mob_passengers` and read in `Game::load_mob` before the riders load; the claims are rebuilt from the ids the riders get (`claims_from`). A vanilla vehicle gets no key; a custom vehicle with nobody on it has a stale list removed. An old save (no key) is seated by `settle`.

### Network

* **Passengers:** unchanged (`SetPassengers` lists every rider; `lan_riding` already built the list from `mount`). A stock client (it sees the proxy entity) gets the list as vanilla sends it and places riders by the proxy's vanilla attachments (no seats; not our code).
* **Seats for Horizon clients:** new `horizon:mob_sync` message **kind 4 `Seats`** (`horizon-protocol/src/mob_sync.rs`): `entity varint, count u8 (<=16), (rider varint, seat u8)*count`, absolute (an empty list frees every seat). Kind numbers are forward compatible: a client that does not know kind 4 skips the message by its length (that rule is the format's own and is tested). No frame version bump. The server sends it when the claims change (`collect`: `Collected.seats`, at once, not rate limited) and after the snapshot to a player that starts tracking the vehicle (`seats_message`, `game/lan_mob_sync.rs`). The client keeps them per entity in `RemoteCustomMob.seats` (newest frame wins, like the hitbox mask) and `Game::apply_remote_seat_claims` copies them onto the vehicle's `CustomState` before `sync_links` each tick, so a remote client places each rider on the seat the server chose. Tests: wire round trip and bounds, collect/once/new viewer, store apply and ordering.

## 3. `seat-set` (the F4M-10 deferred service)

* **ABI id 286**, name `mob-seat-set`, `mob_seat_set(entity: u32, rider: option<u32>, seat: u32) -> result<bool, string>`, permission `service.mobs`. Server catalog **1.4.0 -> 1.5.0**. Regenerated with `cargo run -p xtask -- mod-api` and `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen --lib`; nothing generated was hand edited. Hand written, as for 284/285: the `server_services!` row in `yoake-wasm/src/full.rs`, the SDK wrapper `mobs::seat_set(entity, seat, rider)`, the host methods. No new WIT type name (no `Hand`).
* Semantics: `rider` is another mob by handle; `none` takes whoever sits on the seat off (it cannot board again for 60 ticks, as after any dismount). The service checks the vehicle's published view, the seat index against the type's seat count (an error for a seat the type does not have), that a mob does not ride itself, and returns `Ok(false)` for a dead vehicle; it queues `Command::SeatSet`. The command is applied at the end of the vehicle's tick (`MobEvent::SeatSet` -> `riding::seat_set` in `apply_brain_events`): a taken seat, a rider the seat does not allow, a loop, or an unknown rider change nothing and are logged at debug level. The player cannot be placed by it.
* It is a script's forced boarding: like `/ride` it ignores the boarding cooldown; unlike `/ride` it does not ask native mods (`native_mob_may_ride`), since the event is applied without the `Game` (see open questions).

## 4. Tests

Run through `cargo-locked.sh` at the final commit (all green):

| Suite | Result |
|---|---|
| `horizon-client-lib --lib` | **1906 passed**, 0 failed, 48 ignored (19 new: 3 `cooldown_persists` in `ai/tests.rs`, 14 in the new `ai/tests_4d.rs`, 1 in `lan/mob_sync/tests.rs`, 1 in `custom/remote/tests.rs`) |
| `horizon-client --lib` | **1251 passed**, 0 failed, 52 ignored (resources) |
| `horizon-client --lib --features yoake-hook-bench,yoake-wasm/test-guests wasm_mods::mob_runtime` | **7 passed** (1 new, `the_seat_set_service_queues_its_command_and_is_gated_by_service_mobs`, through the existing wasm test harness's `GameServerHost`) |
| `horizon-mods` | **63 passed** (1 new: `cooldown_persists_defaults_to_false`) |
| `horizon-protocol` | **228 passed** (1 new: `a_seats_message_round_trips_and_is_bounded`) |
| `horizon-mod-api-gen --lib` | **46 passed** (WIT, SDK and adapters regenerated for id 286 / catalog 1.5.0) |

New tests by item: mounting several mob passengers up to the seat count (`several_mobs_board_up_to_the_seat_count`, `the_player_and_mobs_share_the_seats`, `a_seat_can_be_asked_for_and_refusals_say_why`), seat positions (`riders_sit_at_the_offset_of_their_seat`), `allowed_types` (`allowed_types_pick_the_seat`), the driving seat (`the_driving_seat_steers_and_the_others_carry`), sticky seats (`a_seat_is_kept_when_another_rider_gets_off`), dismount on death (`riders_get_off_when_the_vehicle_or_a_rider_dies`), old riders and surplus riders (`the_sync_seats_riders_that_came_without_a_claim_and_drops_the_surplus`), persistence round trip (`the_seats_of_the_riders_survive_a_save`), vanilla one-rider paths unchanged (`a_vanilla_vehicle_keeps_its_one_rider_rules`, `a_custom_type_without_seats_keeps_the_old_link_and_the_ai_boards_seats`), the `seat-set` command (`the_seat_set_command_seats_and_frees_a_rider`) and service (mob_runtime test), the network (protocol, server `the_seats_of_a_vehicle_are_sent_when_they_change_and_to_a_new_viewer`, client `the_seats_of_a_vehicle_reach_the_client_and_the_newest_frame_wins`), `cooldown_persists` both ways (`a_restarted_melee_goal_strikes_at_once_as_vanilla_does`, `with_cooldown_persists_a_restart_keeps_the_attack_cooldown`, `a_kept_cooldown_runs_out_in_game_time_while_the_goal_is_stopped`).

Not run (no game resources here): the `Game` glue (a real click boarding with `player_seat`, `/ride` of several mobs, `put_mob_passengers`/`load_mob` over a real world, `apply_remote_seat_claims`, `lan_mob_sync`) is compile checked only; the pieces they call are the tested pure functions. Bindgen-annotated structs (`Mob`, `Extra`, ...): **not changed** (`MobEvent` is not annotated; seat data is in `CustomState`).


## 5. In-game checks for Yoake

1. A type with 3 seats (seat 0 `controls: player`, `yaw_lock`, seat 1 and 2 `none`) and a `/summon` of three chickens: `/ride <chicken> mount <vehicle>` three times; all three sit at their seats, a fourth is refused (`commands.ride.mount.failure.generic`).
2. Right click the vehicle: the player takes seat 0 while seat 0 is free, else the next seat that allows it; steering works from seat 0 only.
3. Kill one chicken: the other two stay on their seats (no shuffling). Kill the vehicle: all riders drop.
4. Save and reload with riders aboard: the riders are back on the same seats (`passenger_seats` in the vehicle's `yoake` compound).
5. A second window (LAN client, Horizon): riders sit on the same seats as on the host, also when the guest joins late (the `Seats` message follows the snapshot). A stock client sees the riders on the proxy's vanilla seat.
6. `cooldown_persists`: a mob with `interval: 200` and the flag strikes once in 200 ticks even when it loses and regains its target; without the flag it strikes at every restart.
7. A script calling `seat-set` (a WASM guest with `service.mobs`).

## 6. Open questions

1. (Answered in 7.1) **Guests (LAN players) riding a custom vehicle** are not in the claims (`player_seat` is the host window's). A host and a guest on one vehicle could be given the same seat. `interact_custom_mob` used to count a guest as "the player aboard"; the claim-based free seat does not. Does a guest riding a custom mob need seats now?
2. (Answered in 7.2) **Native mods and forced boardings:** `seat-set` and the AI boarding of a seated vehicle do not call `native_mob_may_ride` (the event is applied without the `Game`); `/ride` does. Should they ask the mods?
3. (Accepted, 7.4) **A mob rider steers only from a `player` seat** (the 4c change above). If you want `controls: none` seats to keep a steering mob, `mob_may_steer` is the one place.
4. (Answered in 7.3) **Locators for mob riders (F4M-6):** mob riders use the static `offset` on server and client; the pose-driven locator seat is only wired for the player's seat in the hook that exists. A smooth locator for mob riders needs the render state's locator per seat.
5. (Accepted, 7.4) **Stock clients** see riders by the proxy entity's own attachment points (no seats). Is that enough?
6. `cooldown_persists` covers melee and area attacks only (reasons above). Ranged and leap already keep a cooldown across a restart.
7. The `/ride` path for the player (`self.riding = Some(..)` without a click) does not set `player_seat`; the player takes the first free seat that allows it.

## 7. Follow-up: reviewer decisions (commits after 4d8626d0b)

Answers to open questions 1 to 5 above (3 and 5 are recorded as accepted). No bindgen-annotated struct or function changed: `Mob`, `Extra` and `MobEvent` have the same shape, and `apply_brain_events` keeps its signature and hook id (only its body leaves two event kinds for the game).

### 7.1 Players on seats (decision 1)

* `CustomState.player_seat` is gone. `CustomState.player_claims: Vec<PlayerClaim { player: [i32; 4] (UUID), seat }>` is the reservation of the host in the window and of every LAN guest, next to `seat_claims` (mobs). `riding::claim_player(mobs, vehicle, uuid, seat)` gives a player the seat asked for or the first free seat that allows a player (`allowed_types`), refusing `Taken`, `NoSuchSeat`, `NotAllowed`, `Full`; a player that already has a claim keeps it. `assign_with` takes any number of players, and `board`, `settle`, `mob_rider_seat`, `free_seat_for_player` count them, so no two riders ever share a seat (tests `host_guest_and_mob_riders_never_share_a_seat`, `players_obey_allowed_types_and_each_other`). `board` lost its `player_aboard` flag (the claims say it).
* Who claims: the boarding click (`interact_custom_mob`, the seat the click picked, reserved before `self.riding` is set; a click that cannot be reserved does nothing) and `/ride <player> mount <custom vehicle>` (first free seat; a full vehicle is `commands.ride.mount.failure.generic`). Guests go through the same code in their swapped-in turn (`acting_player_uuid`). A guest may now board a custom vehicle the host rides (`guest_boarded` only bounces it from vehicles without seats).
* Release: `Game::sync_player_seats` (once a tick from `sync_riders`, before the mob riders are settled so players keep priority) calls the pure `riding::sync_players(mobs, riders)`: the claim of a player that no longer rides the vehicle (dismounted, disconnected, died; the game's `riding_players` is the host's `riding` plus every guest with a vehicle that is not dead) is freed, a rider that came without a claim takes one, a player no seat is left for is returned and put off, and the vehicle's rider input is dropped when no claimant steers. Test `a_disconnect_frees_the_seat`.
* Steering: only a player on a `controls: player` seat feeds `Mob.rider` (`tick_mount_riding`), so a guest on a carrying seat no longer overwrites the driver's input.
* Persistence: the vehicle's `yoake` compound gets `player_seats: [{ uuid, seat }]` (players aboard, plus names a load gave to players not back yet); on load they go to `saved_player_seats`, and a player that boards later gets its saved seat while it is free (the `RootVehicle` restore of the host or a guest). Test `the_seat_of_a_player_survives_a_save`.
* Network: `horizon:mob_sync` kind 4 `Seats` is unchanged on the wire; players are listed as their entity id next to the mob ids (a guest by its connection's entity id; the host in the window has no id its guests know, so it is not listed, but it still reserves the seat on the server). The server turns UUIDs into ids in `lan_mob_sync.rs` (`mob_sync::add_player_seats`, `seats_message_with`; `Collected.players` carries the UUIDs). The client splits the list: ids that are mobs become mob claims, its own entity id its own player claim, any other id a player claim under a made-up UUID, so its own seat is placed correctly even when other players or mobs hold earlier seats.

### 7.2 Native veto (decision 2)

`apply_brain_events` now leaves `MobEvent::StartRiding` (when the vehicle has seats) and `MobEvent::SeatSet` in the queue; `handle_mob_events` takes them after the tick, when `self.mobs` is whole again (inside the tick the mobs live in a local list and `native_mob_may_ride` could not read their kinds). `Game::board_seated_mob` (AI boarding, honours the cooldown) and `Game::script_seat_set` plan the change without making it (`riding::plan_board`, `plan_seat_set`), ask `native_mob_may_ride` (boarding: `mounting = true`; `seat-set` with `none`: `mounting = false` for the rider on that seat, as `/ride dismount` asks), and only then `commit_board` / `seat_set`. A vetoed or impossible change changes nothing, and the mods are not asked for what would not happen. Vanilla vehicles keep their old paths with no new question to the mods. Tests (`a_native_veto_stops_seat_set_and_ai_boarding`, `the_seat_set_command_seats_and_frees_a_rider`) use a veto test double (a closure given the rider, the vehicle and `mounting`) in place of `native_mob_may_ride`, since a `Game` cannot be built here; the glue (plan, ask, commit) is the three lines in `game/player_seats.rs`.

### 7.3 Locator seats (decision 3)

* Server: a seat with a `locator` and no `offset` (the schema allows exactly one of them) is `locator_driven`. Its position is the locator's rest position in the model (through the type's model, made at load: half turn about Y, divided by 16) times the mob's scale, the type's `model.scale` times `Mob::model_scale()` (`riding::seat_offset`). A seat with a static `offset` is used as written. `riders::passenger_attachment` (first seat, multiplied by the mob scale afterwards) gets the type scale only, so nothing is scaled twice. Test `the_server_seats_a_locator_seat_at_the_rest_position_times_the_scale`.
* Client: `CustomRender::animated_locator(mob, type, name)` returns the locator of the pose the mob was last drawn in (F4M-6's `MobRender.matrices`), in the same frame. `Game::posed_locators` gives it to `position_rider` (the player's seat, a guest's too) and `position_riders` (mob riders), through `riding::seat_position_of` / `mob_rider_attachment_posed`, times the same scale as the server. Before the first draw (a dedicated server, a mob nobody watches) the rest position is used. The seat is refreshed every game tick from the latest pose and the engine interpolates the rider between ticks, so its render position follows the animation smoothly; the pose is sampled once per tick, not re-evaluated inside a frame. Test `the_client_seats_a_rider_at_the_posed_locator` (a bone moved one block up and 8 px back moves the seat by exactly that, scaled for a baby, rest position before the first draw).

### 7.4 Accepted behaviour (recorded)

* Stock (non-Horizon) clients see riders at the proxy entity's own seat positions (vanilla `SetPassengers` places them by the proxy type's attachments); they get no seats and no animated locators. Accepted.
* Only `controls: player` seats steer for mob riders (`mob_may_steer`); a mob on a `none` seat is carried. Accepted.

### 7.5 In-game checks for Yoake (added)

8. Host and a LAN guest both right-click the 3-seat vehicle: they take seats 0 and 1 (never the same), a chicken added with `/ride` takes seat 2, a fourth rider is refused. The guest leaves the game: its seat is free for the next rider and the host stays where it was.
9. A guest on a `none` seat does not steer; the driver on seat 0 keeps control.
10. Save with a player on seat 1 and reload: the player that rides again takes seat 1.
11. A native mod that cancels `EntityMountEvent`: neither `seat-set` nor an AI mob boards a seated vehicle, nor is a rider taken off.
12. A seat with `locator: "mouth"`: the rider moves with the mouth during the pounce animation (host and LAN client), and a baby of the type sits at half the distance.
