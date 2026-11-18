# MX3REF: read-only hook arguments in the bridges (RefFieldTable from bindgen, dispatch_id_ref from the codemod)

Date: 2026-11-19, on `yoake2/merge` after the toolchain pin (MX3PIN). rustc 1.100.0.

## bindgen: `REF_FIELDS_<type>` next to every `FIELDS_<type>`

Every generated `pub static FIELDS_<mangle>: FieldTable<T>` now has `pub static REF_FIELDS_<mangle>: RefFieldTable<T>`
right after it, with the same getters over `&T` (the generated getters only read fields, so they need no `&mut`).
`FieldTable` itself is unchanged. Glue diff: entities +133 lines, items +81, entity_query +30, game_items the `use`
line; ids, stubs and the lock unchanged. The regeneration gate passes with it.

## Bridge generator: `&T` / `&self` arg0 through `dispatch_id_ref` (agy, reviewed and fixed here)

A hook whose arg0 is `&T` (free fn, call site) or `&self` (method) and whose `T` has a `REF_FIELDS_` table is bridged
with `runtime::dispatch_id_ref(id, p0, &..::REF_FIELDS_.., &mut |p0| ..)`; `p0: &T` goes to `original` unchanged;
result kinds, range checks, observe-only, `install_once`, `BRIDGED`, `mode` are as for `&mut T`. Without a table the
reason is "arg0 is &T, no RefFieldTable for `T`" (or `&self`); a `&Game` arg0 stays unbridged ("no read-only root
dispatch"). The non-reference reason now reads "arg0 is not &T or &mut T". (agy's version did not compile: a borrow
of the replaced type was used after the move; fixed by reading `mutability` first.)

Coverage: **entities 81 bridged (was 26), 59 unbridged**: 31 `&T`/`&self` whose type has no scalar-field table
(`World`, `BlockFacts` and similar engine context), 24 whose arg0 is not a reference, 4 `&mut T` without a table.
`game::item_use` unchanged (29 / 35).

## Tests

- `yoake2_bridge::a_read_only_hook_reads_fields_cannot_write_them_and_can_replace_the_result`: hook 40
  (`Experience::death_reward(&self, bool) -> u32`, Full): the guest reads `total` through the shared handle (get
  works), its `set` is refused with `7` (ReadOnly), it returns `total * 2` and that replaces the result; the argument
  is unchanged.
- codemod `tests/bridge.rs`: a `&T` free fn, a `&self` method and a `&T` call site with `REF_FIELDS_` tables bridged
  through `dispatch_id_ref`; `&T`, `&self` without a table and `&Game` unbridged with their reasons.

| suite | result |
| --- | --- |
| hook crates | 129 passed |
| `horizon-client --lib` off / `yoake2-slice` | 1193 / 40 ignored; 1195 / 44 ignored |
| `horizon-client-lib --lib` off and on | 1619 / 48 ignored |
| horizon-mods, yoake-wasm | 44, 124 |
| codemod `check --callsites`, `bridge --check` | clean |
| yoake2_glue / engine_call / slice / bridge | 31 passed, 1 ignored |
| yoake2_loader, yoake2_examples | 19, 2 |
| yoake-slice (`wasm`), yoake-guest-sdk | 77; 13 / 1 ignored |
| yoake-bindgen mini_engine; **regeneration gate** | 8; **passed** |
