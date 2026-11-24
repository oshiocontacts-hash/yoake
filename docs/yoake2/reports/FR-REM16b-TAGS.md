# FR-REM16b lane TAGS: damage-type and enchantment tags

## What

- Reused the content root's existing load-order `TagMerge` for damage-type tag files. Merged
  `data/<ns>/tags/damage_type/*.json` files now also update the prepared `damage-tags.json` bundle,
  including `replace`, nested `#tag` references and optional entries. The bundle is untouched when
  no mod supplies a damage-type tag.
- `horizon-items::enchant::Enchantments::load_with` now uses the shared multi-namespace tag loader.
  Vanilla and mod enchantment tags therefore drive `exclusive_set`, `in_enchanting_table`, and the
  other tag consumers. Its damage-type and entity-type tag maps also see content-mod namespaces.
- Configuration-phase registry tag sync reads the merged vanilla, convention, and content-mod
  namespaces, expands cross-namespace nested tags, and sends mod-owned tags as well as additions to
  vanilla tags. With no content-root marker it reads only `minecraft`, as before.

## Files

- `crates/horizon-client/src/mod_content.rs`: damage-tag bundle merging and content identity inputs.
- `crates/horizon-items/src/enchant.rs`: shared merged-tag loading for enchantments and effect tags.
- `crates/horizon-client/src/lan/data.rs`: multi-namespace registry tag packet construction.
- `crates/horizon-client/src/mod_content_tests.rs`: content-root merge, nested/optional entries,
  replacement, damage catalog, and byte-identical no-tag output fixtures.
- `crates/horizon-items/src/enchant.rs`: mod `exclusive_set` incompatibility and enchanting-table
  candidate fixture.
- `crates/horizon-world/src/damage_reduction.rs`: armor-bypass damage calculation fixture.
- `crates/horizon-client/src/lan/data.rs`: merged network tag and no-mod baseline fixture.

## Tests

Tests were added but **not run**, because this lane forbids shell use. All fixtures use small
temporary JSON trees and need no Minecraft resources.

- Content root: merge with and without `replace`, a vanilla tag referencing `#ruby:shards`, absent
  optional entries, `Catalog` membership, and unchanged `damage-tags.json` without a damage tag mod.
- Damage reduction: `ruby:shatter` in `minecraft:bypasses_armor` leaves 10 damage unchanged and does
  not hurt armor; the ordinary control is reduced to 4 by 20 armor.
- Enchantments: `#ruby:gems` makes `ruby:keen` and `ruby:wise` incompatible, and both occur in the
  available enchanting-table results through `#minecraft:in_enchanting_table`.
- Network: a nested mod tag resolves to the synchronized mod registry id, an absent optional id is
  omitted, and the no-content-root packet keeps its vanilla baseline.

## What is left

Claude needs to run formatting, the focused tests, and the repository self-check listed in
`.lane/TASK.md`. No known feature work remains in this lane.
