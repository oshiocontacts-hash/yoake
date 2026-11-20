# FR-F1 wave 2: W7 (build-time stamping overlay) and W4 (private self types, destructuring parameters)

Branch `yoake-free/f1-w2` (cut from `yoake-free/f1-w1` @080b2dd58) in `C:\Users\USER\NestgateHorizon\horizon-f1`. Not pushed.
Builds used `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0` except where incremental behaviour was the thing measured (the end-to-end
fixture also ran with 0: cargo's own fingerprints, not rustc's incremental cache, are what the overlay has to get right).
Disk was 28 GB free at the start and 26 GB at the end (two other builds), below the 30 GB the brief asks for before a full stamped build,
so **the release gate (fast-profile `horizon-client` build, stamped against unstamped) was not run**. Everything else was done with
`cargo check` or small fixture builds (at most 1.5 GB). The exact commands for the gate are in section 3.

| Item | Result |
|---|---|
| W7 plain `cargo build` unstamped, `YOAKE_STAMP=1` stamped overlay | done, end-to-end tested on a throwaway workspace |
| Ids identical across clean runs | done: fixture (two target dirs) and the real tree (two overlay roots: 1,039 files, byte-identical) |
| `file!()`, panic locations point at original files | done (tested) |
| Fingerprints: unrelated edit does not rebuild the stamped crate | done (tested), plus source edit and ledger edit do rebuild it |
| `apply --verify` against the overlay | done (`--overlay`), end-to-end tested with a stamp that only fails when stamped |
| Release gate: `--verify-arg build` once, time and size vs unstamped | **waiting for disk** |
| W4 private self types, destructuring parameters | done; three crates verified through the overlay: clean in round 1 |

## 1. W7: the overlay

Commits: `W7: rustc-wrapper overlay ...`, `W7: verify through the overlay ...`, `tools: keep *.sh LF`, `Ledger-only commit ...`.
Files: `crates/yoake-hook-codemod/src/overlay.rs` (new), `main.rs`, `run.rs`, `verify.rs`, `tests/overlay_e2e.rs` (new), `tools/yoake-stamped-cargo.sh` (new), `.gitignore`, `.gitattributes`.

### How it works

* **One binary, three roles.** `yoake-hook-codemod` is also the rustc wrapper: when its first argument is a `rustc` path it runs in wrapper mode,
  otherwise it is the codemod as before. `RUSTC_WORKSPACE_WRAPPER=<exe>` makes cargo call it for workspace crates only (never for registry crates).
* **Off by default.** Without `YOAKE_STAMP=1` the wrapper (if it is configured at all) execs rustc unchanged; and nothing configures it: a plain
  `cargo build` has no wrapper and writes no overlay. `tools/yoake-stamped-cargo.sh <cargo args>` builds the codemod once (release, in `target-tools/`),
  sets `YOAKE_STAMP=1`, `RUSTC_WORKSPACE_WRAPPER`, a **separate** `CARGO_TARGET_DIR` (`target-stamped/`; sharing a target dir with the plain build would make cargo
  rebuild everything on every switch because the wrapper is part of its fingerprint) and `YOAKE_STAMP_DIR`, then runs cargo.
* **What is stamped.** A crate that has a line in `hook-ledger/stamp-config.txt` and whose **library root** is the file rustc is asked to compile.
  Build scripts, bins, examples, other crates, and a crate without a config line go through untouched. If anything fails while building the overlay, the crate
  is built unstamped and a `yoake-stamp: <crate> is built UNSTAMPED: <reason>` line is printed (never a silent change).
* **Overlay.** The crate's source closure (root file, module files found with the codemod's own module resolver incl. `#[path]`, files named by literal
  `include!`/`include_str!`/`include_bytes!`, `Cargo.toml`) is copied to `<target>/yoake-stamp/root/<same path relative to the workspace>` and stamped there by the
  normal `apply` code with a **frozen ledger**: no new ids, no ledger writes, a fn without a live ledger id is simply not hooked, a live id whose fn became
  ineligible is left alone. Relative `#[path]`/`include` paths keep working because the layout is the same as in the workspace.
* **Paths.** The overlay file is passed to rustc by absolute path together with `--remap-path-prefix <overlay root>=` (empty target). Measured with rustc 1.100.0:
  an empty target gives exactly `crates\x\src\a.rs` for `file!()`, panic locations, debug info and diagnostics, identical to the unstamped build (a relative input would print forward slashes and a target of `.` a leading `.\`).
* **Fingerprints.** Cargo decides what to rebuild from the dependency file rustc writes. The wrapper rewrites it after a successful compile: overlay paths become the original source paths and
  `hook-ledger/<crate>.json`, `stamp-config.txt` and `stamp-allow.txt` are added as dependencies. So an edit of the real source or of the ledger rebuilds the crate, anything else does not.
* **Speed.** The overlay is rebuilt only when something it reads changed (a stat list of the closure, the ledger shard, `meta.json`, config, allow/exclude files and the codemod binary is stored in `.state/`). Measured on the real tree with a **release** codemod: all three crates cold (closure scan, copy, stamp): **30.1 s**; warm (nothing changed): **6.2 s** (it still parses the module tree to know the closure; a cached file list could take this to about 1 s, not done). Debug codemod: about 94 s per cold run of the three crates.
* **Locking.** A per-crate lock directory serialises two rustc invocations of the same crate (`check` and `build` of lib and tests).

### New codemod commands and flags

* `sync-ledger --workspace-root W [--crate N]... [--frozen] [--overlay-root O]`: stamps every configured crate in a **scratch** overlay with a writable ledger: ids and exclusions land in `hook-ledger/`, the sources are not touched. This is the "ledger-only commit" step for a bot or a developer who added functions. `--frozen` builds the overlays exactly as the wrapper does (used for the determinism check).
* `apply --frozen-ledger` (read-only ledger), `apply --verify --overlay [--verify-dir W]`: see below.

### `apply --verify --overlay`

Each round = `sync` (overlay with writable ledger: ids, `stamp-exclude.txt` requests become manual exclusions, retired ids) then `cargo <verify-args>` with the wrapper environment
(`YOAKE_STAMP_TARGET` or `<verify-dir>/target-stamped`). Compiler errors name the original paths (remapped), the line numbers are those of the **stamped** overlay file, so
attribution reads the overlay copy. The result is a changed ledger and an unchanged tree (tested: `git status` shows only `hook-ledger/`).
A stamp that only fails when stamped (a method whose hand-written `__yoake_orig_foo` collides with the generated one) is withdrawn in round 1, the build is clean in round 2, the id is a tombstone, the exclusion is manual, the source file is byte-identical.

### Tests (`crates/yoake-hook-codemod/tests/overlay_e2e.rs`, `#[ignore]`, run with `-- --ignored`; 2 tests, 69 s)

Fixture workspace of three crates (`demo` stamped, `other`, `app`), real hook crates as path dependencies:

| Check | Result |
|---|---|
| plain build: no overlay, unstamped | pass |
| `sync-ledger`: ids appear in `hook-ledger/demo.json`, the source file is byte-identical | pass |
| stamped build from the committed ledger; overlay file contains the attributes, the workspace file does not | pass |
| `file!()` printed by the binary and the panic location seen in the panic hook equal the plain build's (`crates\demo\src\a.rs`) | pass |
| behaviour with no mod unchanged (`bump()` result) | pass |
| a second clean build in another target dir gives a byte-identical overlay tree | pass |
| nothing changed: nothing recompiled | pass |
| edit of the unrelated crate `other`: `other` and `app` rebuild, `demo` does not | pass |
| edit of `demo`'s source: `demo` rebuilds, the overlay follows, a new fn without ledger id is not hooked | pass |
| edit (touch) of `hook-ledger/demo.json`: `demo` rebuilds | pass |
| overlay verify: withdrawn stamp, tombstone, manual exclusion, tree untouched | pass |
| exe size of the fixture, plain / stamped | 1,253,987 / 1,271,206 bytes (+1.4 %, 2 hooks) |

Unit tests added: include scan, wrapper detection, input-file detection, dependency-file rewrite (6 forms of the path). All codemod, macro, rt and ledger tests pass
(`cargo test -p yoake-hook-ledger -p yoake-hook-macro -p yoake-hook-rt -p yoake-hook-codemod`, 140+ tests).

### On the real tree

* `hook-ledger/` now holds the committed ids for the three stamp-config crates (`next_id` 8,475, three shards 3.7 MB), produced through the overlay (**ledger-only commit**); no source file changed.
* Determinism: two independent `sync-ledger --frozen` runs into two overlay roots give **1,039 files, all byte-identical**; the overlays contain 8,101 hook attributes. This is the "same ids on two machines" check without a compile; with the fixture it was also checked after real builds.

### Limits and risks

* Dependency changes of the **codemod itself** (a new codemod build) re-stamp (the exe's stat is in the state) but do not trigger a cargo rebuild of an unchanged crate; when the stamping rules change, clean the stamped target dir.
* `include!`/`include_str!` with non-literal paths (`concat!(env!("OUT_DIR"), ...)`) are not copied (OUT_DIR is cargo's); a crate that includes a workspace file by a computed path would fail in the overlay and fall back to unstamped with the message above.
* Two cargo invocations at the same time on the same stamped target dir are serialised per crate by the lock, not across crates sharing a directory (disjoint file sets).
* `YOAKE_WORKSPACE` can override the workspace discovery (it walks up to the first `hook-ledger/stamp-config.txt`).
* The release gate and the incremental behaviour on `horizon-client` itself (rather than on the fixture) are not measured yet.

## 2. W4: private self types and destructuring parameters

Commits: `W4: --private-types ...`, `W4: trait wrappers name destructuring ...`.

* **No crate-level path** (4,090 fns in the spike): the exclusion existed because the typed registration `Type::__yoake_register_x` is `pub(crate)` and the bridge generator names types by crate path. Hooking
  itself needs neither. New switch `--private-types` / stamp-config `private_types=true` (default off for the CI `check` over `modules.txt`, on for the three crates). The bridge keeps listing such hooks as unbridged (it has no field table for them), so no generated code names a private type. Note: those stamps are only reachable through a per-module bridge `include!`d inside the owning module (design in the spike); a guest cannot hook them yet.
* **Destructuring, `_`, `ref`, `@` parameters** (`208` in the spike plus trait impls): the macro gives such parameters a synthetic name on the outer fn (`__yoake_pat_N`); the original body keeps its pattern in `__yoake_orig_*`. For converted trait methods the codemod renames the pattern in the **wrapper** (`__yoake_aN`) and forwards it; the helper keeps the original signature. `eligible.rs` no longer refuses these.
  The first overlay verify exposed the missing half: 39 trait wrappers were generated with fewer arguments than parameters (E0061: the wrapper only forwarded identifier parameters). Fixed and covered by a codemod test (conversion, then idempotent second run).
* Tests: `yoake-hook-rt/tests/mechanics.rs` (tuple, struct pattern and `_` parameters, hook replaces arguments), macro unit test, codemod `--private-types` test.

### Verified through the overlay (`apply --stamp-config ... --verify --overlay --verify-arg check`)

| Crate | verify time | rounds | stamps withdrawn | hooks now | new ids | top exclusions |
|---|---|---|---|---|---|---|
| horizon-world | 1 m 34 s | 1 | 0 | 856 | 856 | small body 1,646; generic impl 1,599; generic type params 619; impl Trait arg 194 |
| horizon-client-lib | 3 m 12 s | 1 | 0 | 3,568 | 3,188 | small body 3,778; impl Trait arg 199; borrowed return 176; by-value self 173 |
| horizon-client (`yoake2-slice`) | 4 m 36 s | 1 | 0 | 3,744 | 3,721 | small body 4,713; borrowed return 275; impl Trait arg 159; trait impl on non-inherent type 82 |
| **total** | 9 m 22 s | | **0** | **8,168** | 7,765 | |

Wave 1 had 5,911 hooks with the same size rule (`min_stmts=3`): **+2,257 (+38 %)**. "Self type has no crate-level path" is gone from the exclusion lists; the remaining large classes are
the size rule (about 10,100 fns, a setting), generic impl/type params (about 2,400), impl Trait arguments, borrowed returns, by-value self.
Before the wrapper fix of the trait wrappers, the first run on horizon-world withdrew 39 stamps over 8 rounds (all E0061) and did not converge in 8 rounds: this is what the round limit and "stamps withdrawn" counter showed, and the reason the loop's default round limit is now 20 in the script.

## 3. Release gate (not run: disk) and how to run it

Needs about 5 GB per build with incremental off and no debug info; run one after the other and delete the first target before the second:

```
# unstamped baseline (fast profile)
set CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0
cargo build --offline --profile fast -p horizon-client --features yoake2-slice        # time it, ls -l target/fast/horizon-client.exe
# stamped, from the committed ledger (this is the gate: it is `build`, not `check`)
tools/yoake-stamped-cargo.sh build --offline --profile fast -p horizon-client --features yoake2-slice
#   or, to also withdraw rejected stamps:  apply --stamp-config ... --verify --overlay --verify-arg build ...
```

What I can say without it: `cargo check` of the stamped overlay is clean (section 2); the codemod part of a clean stamped build costs 30 s (release codemod, three crates), 6 s when nothing changed; the spike's stamped build with 14.7 k hooks was +17 % binary and +70 % clean build, so with 8.2 k hooks and a fast codegen profile expect about half of that. **That is an estimate, not a measurement.**

## 4. Needs checking

* The release gate numbers (time, size) and a smoke run of the stamped `horizon-client.exe` with no mod.
* Incremental behaviour on `horizon-client` itself (fixture only so far), including rust-analyzer: it sees the plain sources because no wrapper is set in the repository; hook ids are in `hook-ledger/` only.
* Debug info for stamped crates: `file!()` and panic locations are verified; debugger breakpoints on overlay-compiled code were not checked (paths in debug info are remapped the same way).
* The bridge for private-type hooks (per-module bridge inside the owning module) is design only.
* New functions need a ledger id before they are hooked: `sync-ledger` must run in CI or by a bot (the `check` job should call `sync-ledger` and fail on a diff, like the existing `check`).
