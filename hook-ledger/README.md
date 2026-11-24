# hook-ledger

The committed hook id tables: one shard per crate (`horizon_client.json`, ...) and `meta.json`.
Ids are global, append-only and never reused (`yoake-hook-ledger`, `check_append_only`).

## `meta.json`

```json
{ "version": 2, "next_id": 11755,
  "reserved": [ { "first": 1040000, "last": 1040999, "owner": "custom mob hooks (FR-F4-MOB), assigned by hand" } ] }
```

* `next_id` is where the allocator (`LedgerSet::assign`, used by `sync-ledger` and the stamp tool) hands out the next id.
* `reserved` is a list of id ranges the allocator **skips** (it jumps past the range). A range is for hooks that are
  assigned by hand, one per new hook, by a feature whose ids must not move (the custom mob hooks, FR-F4-MOB).
  Keep reserved ranges far above `next_id` (and below `TEST_ID_BASE`, 1,048,512), so ledger waves never get close;
  if `next_id` ever reaches one, the allocator skips it instead of colliding.
* Validation (`LedgerSet::validate`, run on every load and save) fails when an entry id lies inside a reserved range,
  when a range is empty or outside `1..TEST_ID_BASE`, or when two ranges overlap. The test
  `horizon-mods: the_reserved_id_ranges_are_in_the_locks` pins the custom mob range.
* To reserve a range: add it to `meta.json` (or `LedgerSet::reserve`), never one that already holds an entry.

## When the source moved (renames and tombstones)

A live key whose function moved or went away stops `sync-ledger` (and the stamped build, which would build that
crate unstamped) with `missing live ledger keys`. Fix it through the overlay route, never by hand-editing the JSON
and never by rewriting sources in place:

```
yoake-hook-codemod sync-ledger --workspace-root . \
  --rename OLD_KEY=NEW_KEY     # the function moved or was renamed: the id stays on the new key
  --tombstone KEY              # the function is gone, or is no longer hookable: the id is retired for good
```

Both flags can be repeated and may name keys of any crate (each crate only applies its own). A rename whose new
key is not hookable (a generic impl whose parameters are not `'static`, say) is refused with a hint to tombstone.
New functions get their ids in the same run. Afterwards `check-history --base <old ledger dir> --current hook-ledger`
must still say `ok` (ids are never reused or moved to another key without a rename).
The flags are also read by `apply --verify --overlay` (`--rename`, `--tombstone`).
