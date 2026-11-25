# FR-F4M-15: custom-mob acceptance (the ten scenarios)

Runner: `tools/custom_mob_acceptance.py` (added in 3f6be5f8a2). It prints one JSON object with `s1`..`s10`, each `{pass, detail}`; `--only s5,s6` runs a subset.

Run on yoake-free/merge f0a39f4cf4 (with F4M-5a), debug client, guests fox_ai + fox_texture, release `custom_mobs_200` prebuilt.

| # | Scenario | Full run | Reruns | How it is checked |
|---|---|---|---|---|
| 1 | data only | pass | | render e2e data checks on a data-only package |
| 2 | model formats | pass | | geo.json vs .bbmodel pose equality, 30-frame golden pose table |
| 3 | behaviour | pass | | pounce on a summoned chicken, hit-frame timing, loot, tame / sit / stand / follow, render e2e wasm checks |
| 4 | persistence | pass | | save/quit/reload with sitting, warmth, owner; open without the mod, then with it again |
| 5 | multiplayer + auto-fetch | fail (3 ticks apart) | pass, pass | host + 2 clients, one fetches the .yak; same animation tick on all three |
| 6 | vanilla client proxy / refusal | fail (no proxy fox seen) | pass, pass | stock-brand probe sees `minecraft:fox` (proxy); without `proxy` it is refused in configuration with the content-mismatch text |
| 7 | hot reload | pass | | render e2e reload |
| 8 | debug view | pass | | render e2e debug |
| 9 | performance | pass | | release `custom_mobs_200` against the section 8 budgets |
| 10 | parity goldens | pass | | goldens A-F with the mod uninstalled |

## Fixes during acceptance
- s3 (3439626d69): the pounce check counted chickens spawned with newly loaded chunks; it now looks only at the summoned chicken.
- s6 needed engine work: F4M-5a (4c0741f546, report FR-F4M-5a.md) adds `lan/mob_proxy.rs`, the stock-client type mapping and the brand gate for unproxied custom mobs.

## Left
- s5 and s6 failed once in the full run, which came straight after the whole custom-mob e2e set on a busy PC, and passed in both reruns. Their waits are timing-sensitive under load: s5 compares one sampled tick, and s6's probe may leave before the proxy fox is tracked. Make both condition-based.
- s10 test_d has no pre-F4M bench baseline JSON, so only the goldens are compared.
- The data e2e `strike_mob` check is still flaky in the merge runs.
