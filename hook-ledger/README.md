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
