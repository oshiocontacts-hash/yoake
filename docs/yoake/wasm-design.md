# Yoake WebAssembly mod architecture

Status: design proposal; no implementation is implied.
Decision context: 2026-11-14.

## 1. Intent and historical context

Yoake should run private mod code as WebAssembly components inside `.yak`
packages, using the one signed game executable. The launcher installs and
explains packages; it does not execute their code. The engine exposes a stable,
versioned interface for hooks and host services so ordinary mod releases do not
require rebuilding the game or launcher.

The target order is yasagashi first, then kagerou, replay and kigurumi, then
Nestgate including its Rust client and voice. The end state covers the full
native hook catalog and the host services those mods require. The rollout is
incremental: it must not imply that all hooks are available in W1.

Upstream Horizon previously supplied a Wasmtime-based code-mod runtime and SDK;
the old documentation describes a component interface, Cranelift, and historical
empty-hook and load measurements (`horizon/docs/mods.md:132` at `708f0b48b^`).
It later removed game integration in `e7614d219` and the runtime and SDK in
`0c9a49390`, moving to built-in native hooks. These commits establish the change,
not the maintainers' complete reasoning. No unverified explanation of why they
made that choice should be treated as fact.

Yoake has a different product requirement: separately released, private mod
code must run without shipping unsigned native DLLs or rebuilding the signed
engine for each mod change. This proposal limits the costs upstream might have
had to weigh through subscription-only dispatch, staged host services, explicit
compatibility rules, isolated failures and a measured no-mod baseline. Whether
those measures are sufficient is a release gate, not a proven result.

Vanilla 26.3 remains the specification. With no Wasm mod installed, the game
must retain vanilla behavior and show no measurable performance regression;
Horizon explicitly requires vanilla parity (`horizon/AGENTS.md:9`). Existing
native mods and resource packs are not silently converted or removed.

## 2. Runtime and isolation

### Choice

- Embed Wasmtime 48 LTS, the component model and Cranelift JIT in the game.
- Pin and audit the exact Wasmtime release and enabled features in the engine.
- Treat a guest as a component with a typed interface, not an arbitrary native
  DLL, a precompiled native object, or an untyped core-module ABI.
- Instantiate a separate store and resource budget for each mod instance and
  side. Client and server components do not share guest memory.
- The engine, not the launcher, validates and instantiates components.

The upstream measurements cited above are historical reference points, not
Yoake targets or results: approximately 85 ns for an empty hook and 50 ms for
compile/load. Real calls include argument conversion and mod work; measure
release builds on representative machines and workloads before committing to
performance budgets.

### No-mod fast path

When no Wasm component is installed, do not create a Wasmtime `Engine`, store,
JIT mapping, cache entry, epoch timer, guest scheduler or Wasm dispatch call.
The signed executable still contains the runtime, so its file and download size
can grow even on a vanilla-only installation. Existing native and vanilla paths
must remain correct; benchmark the actual hook-dispatch path rather than claim
that a new branch has literally zero CPU cost.

Record baseline startup time, idle memory, frame times, tick times and affected
hook throughput before integration, then compare the same scenario and hardware
without a Wasm mod. Any meaningful regression blocks release pending correction
or an explicit owner decision. Do not enable experimental runtime work merely
because the binary contains it.

### Budgets and scheduling

- Start with 64 MiB maximum linear memory per component instance, including
  growth; cap tables, instances and other Wasmtime resources separately.
- Meter guest instructions with fuel, replenished per callback or scheduled
  work item, and enforce an epoch-interruption deadline as a second guard.
- Proposed starting budgets are 2 ms for an ordinary synchronous client hook,
  5 ms for a synchronous server hook and 10 ms for a scheduled work item;
  calibrate on supported low-end hardware before enforcing these values.
- Fuel and epoch interruption do not bound native host-service work. Host calls
  need their own bounded input sizes, operation quotas, cancellation and
  nonblocking or explicitly scheduled implementations.
- Do not perform unbounded compilation, I/O, DNS, voice processing or blocking
  work on render and server-tick threads. Use bounded workers and queues.
- A trap, timeout or budget exhaustion is attributed to the instance. Stop that
  invocation and apply a documented disable/retry policy; never unwind through
  game code. Cancellation does not pretend that already committed effects were
  rolled back.
- At mod boundaries, validate lengths, UTF-8, handles, ownership, game state and
  side. Cap callbacks, concurrent jobs, events and queued responses.

Synchronous hooks that must decide an action run with a deterministic fallback
if the mod fails or times out. Each hook's metadata must define its fallback,
ordering, valid phase and threading. Observe hooks cannot mutate the decision.
Hosts enforce authority and permissions on every call, not only at loading.

### Compile cache and executable cost

Compile verified component bytes off critical threads. Cache compiled artifacts
on disk by component SHA-256, engine and Wasmtime versions, target/CPU features,
compiler settings and interface fingerprint. Invalidate on any mismatch and
allow cache eviction; cache misses fall back to recompiling verified Wasm.

Wasmtime serialized artifacts contain executable native code. A matching hash
stored alongside an artifact is not authentication: an attacker who can replace
both can supply native code. Reuse serialized artifacts only after a reviewed
design authenticates them with an appropriately protected per-install key and
filesystem permissions, and only under that design's explicit local-attacker
threat model. Until then, do not deserialize disk artifacts; recompile instead.
Never accept a mod-supplied precompiled artifact. Verification failure means
discarding the artifact and recompiling, not executing it.

Planning allowance only, not a measured estimate: the stripped signed executable
may grow by roughly 10-50 MiB, and a clean release build by roughly 2-15 minutes.
Actual results can fall outside those ranges depending on features, host and
linking. W1 measures binary/download size, clean and incremental build time,
cache behavior and load latency. The JIT and executable-memory policy must be
validated with Windows signing and deployment requirements before release.

## 3. Generated interface and compatibility

### Catalogs and generated bindings

Extend the existing generator rather than maintaining a hand-written parallel
hook API. It already reads hook definitions and emits native ABI bindings
(`horizon/crates/horizon-mod-api-gen/src/build.rs:24`,
`horizon/crates/horizon-mod-api-gen/src/build.rs:54`); its current ABI subset
is explicitly selected (`horizon/crates/horizon-mod-api-gen/abi-subset.json:3`).

Add reviewed metadata to the full native hook catalog and a new host-service
catalog. Generate versioned WIT worlds and types, guest bindings, host adapters,
registration identifiers, capability requirements, compatibility fixtures and
reference documentation from those catalogs. Generator checks must detect
duplicate identities, missing metadata and unreviewed incompatible changes.

Every hook needs an explicit mapping: side, phase, argument/result types,
ownership, threading, ordering, mutation and cancellation rules, failure
fallback and required capabilities. Native references, closures, borrowed Rust
objects and engine internals cannot simply be exported as WIT. Represent them
as bounded value snapshots, typed resource handles or purpose-built operations,
with validated lifetimes and no exposed pointers. A hook is unavailable to Wasm
until its semantics and conversions have been reviewed and tested.

The full catalog is the destination, not a promise that every native callback
is representable without design work. Avoid a generic reflection or raw-memory
escape hatch: either would make safety and compatibility impossible to audit.
The generator emits a machine-readable availability and feature manifest so a
mod can fail clearly when a required hook or service is not yet implemented.

### Registration and dispatch

At initialization a component explicitly subscribes to named, versioned hooks.
The host validates the requested subscriptions and capabilities, then installs
per-hook subscriber lists. Invoke guest code only for subscribed hooks. With no
subscriber, do not construct guest values, enter a store or make a Wasm call.
The hook path must avoid Wasm-specific work entirely in the no-mod configuration.

Specify subscription lifecycle, stable ordering relative to native listeners,
priority conflicts, reentrancy and unsubscription during dispatch. Mutations are
applied through typed operations only at defined safe points. Snapshot data must
remain valid for the documented call lifetime; handles are revocable and cannot
outlive their owner or world. Client and server subscriptions are separate.

### Interface versions

Use a semantic interface version independent of the package and game versions.
Within a major version, minor releases are additive: existing functions, types,
meaning, fallbacks and permission requirements remain compatible. Never silently
repurpose an identifier or change an existing function's behavioral contract.
Deprecation is documented; removal or incompatible semantics require a new major.

The manifest declares a minimum interface version and any required named
features. Loading checks the major, minimum minor, side and features before
instantiation and reports a precise error. A newer engine must continue accepting
the old supported interface; an older engine must reject a missing requirement
rather than start a partially working mod.

Additive source changes to WIT are not automatically binary-link compatible.
Freeze each published world and import/export signature, retain old worlds and
linkers, and adapt them to current implementations through explicit shims. New
features are separate versioned interfaces or new worlds. Keep actual previously
compiled guest fixtures, and test them against every later supported engine.
Publish a support and security-fix policy before declaring a version stable.

## 4. Permissions and sandbox

The component receives no ambient filesystem, sockets, microphone, process,
environment, shell or operating-system access. Do not attach a general-purpose
WASI implementation by default. Only explicitly linked host services are
available, and their entry points enforce granted capabilities.

Illustrative `yak.toml` additions, not current syntax:

```toml
interface = 1.0
features = [hook.player-interact, service.commands]

[permissions.client]
required = [ui.inventory, audio.play, voice.microphone]
optional = [network.outbound]

[permissions.server]
required = [commands.register, world.read, world.modify]
optional = [files.data]

[permissions.network]
outbound = [https://api.example.invalid:443]

[permissions.files]
scope = mod-data
```

Catalog capability names and scopes rather than hard-coding the example as the
final set. At minimum cover outbound network, microphone and voice, a mod-owned
data directory, commands, UI, sound, player operations, world access and block
entity/map operations. Define which capabilities are client, server or both;
split read and write, and grant the narrowest meaningful operation and scope.

On installation the launcher shows publisher/identity, package version, side,
requested capabilities, network destinations, required versus optional access
and an understandable consequence for denial. The user explicitly grants or
denies permissions; a server administrator independently controls server grants.
Required denial prevents that side from loading; optional denial must be
observable by the mod and handled without an implicit privilege escalation.

Bind grants to mod identity, approved scopes and the exact package digest. Prompt
again when code, version or requested privileges change; support review and
revocation. The game enforces the same policy even if packages are installed or
modified without the launcher. Permission display is not a security boundary.

File operations are confined to the mod's own data directory, with normalized
paths, symlink/reparse-point protection, quotas and no access to other mods'
data. Network access is brokered with destination, protocol, size, concurrency
and rate limits; validate redirects and DNS results against rebinding and local
address policy. Voice requires a separate explicit microphone grant, a visible
recording state and a bounded host API, not raw device or socket access.
Commands and world mutations remain subject to server authority and player
permissions. No capability supplies credentials or native code execution by
default.

## 5. `.yak`, integrity and multiplayer

### Package format

Introduce an explicitly versioned format revision; do not silently reinterpret
the existing format. Current manifest parsing rejects unknown fields and enforces
its format version (`horizon/crates/horizon-mods/src/yak/manifest.rs:73`,
`horizon/crates/horizon-mods/src/yak/manifest.rs:119`). New readers continue to
accept existing format-1 packages; old readers reject the new format cleanly.
The current client recognizes an older Wasm payload shape without running it
(`horizon/crates/horizon-client/src/yak_mods.rs:153`); naming a file in a
package does not establish runtime support.

Illustrative new archive and manifest layout:

```text
yak.toml
bin/wasm/yasagashi-client.wasm
bin/wasm/yasagashi-server.wasm
assets/...
data/...
```

```toml
format = 2
id = yasagashi
version = 1.0.0

[bin.wasm]
client = bin/wasm/yasagashi-client.wasm
server = bin/wasm/yasagashi-server.wasm
```

The manifest also carries the interface requirements and per-side permissions
described above. This is a proposed shape, not a claim about today's parser.
Payloads are target-independent components: the same Wasm bytes work on each
supported host. Client-only, server-only and dual-sided packages have explicit
side declarations. Single-player runs the relevant client and integrated-server
instances separately. A dedicated server must load only its server component;
today's client loader skips dedicated-server payloads
(`horizon/crates/horizon-client/src/yak_mods.rs:197`).

Reject duplicate or ambiguous entries, traversal, symlinks, oversized archives,
decompression bombs, mismatched sides and malformed components before loading.
Validate manifest/component agreement and bound decompression and extraction.

### Catalog signature and local integrity

The signed catalog authenticates the package identity, version and complete
archive SHA-256; the Wasm is inside those covered bytes. Existing catalog code
already models and checks package hashes
(`horizon/crates/horizon-mods/src/yak/catalog.rs:49`,
`horizon/crates/horizon-mods/src/yak/catalog.rs:126`). No Windows Authenticode
signature is required on each `.wasm`; the executable remains the signed native
program. Existing packaging behavior and its signature assumptions must be
retained for format 1 (`horizon/crates/cargo-horizon-mod/src/yak.rs:48`).

At load, the game verifies a trusted catalog signature and the exact archive
digest itself, then loads bytes from that verified immutable archive or verifies
an authenticated per-file digest list. A launcher-only check or a mutable
extraction directory creates a time-of-check/time-of-use gap. Atomic installation
and file replacement rules must preserve that guarantee across concurrent
updates. Reject modified, unlisted, unsigned or revoked production packages;
allow developer overrides only as a clearly separate opt-in policy.

Signing proves origin and integrity, not safety or confidentiality. A `.yak`
and its Wasm are inspectable by recipients: keep the mod source repository
private, but do not promise that distributed code is secret. Confirm SignPath's
actual policy permits this executable's JIT and its executable-memory behavior.

Keep mod source and its build pipeline private; publish only the approved engine
source for the signing workflow. Build `.yak` files in the private pipeline and
distribute them through the approved catalog. Keep catalog and code-signing
private keys outside public repositories and artifacts; verification keys may be
public (`reports/SP1.md:8`). Review the exact public tree and reachable history:
removing a sensitive file from the current tree does not remove old commits
(`reports/SP1.md:60`).

### Join compatibility

For a negotiated Yoake modded session, the server advertises each required
both-sided mod's ID, exact version and SHA-256 of the complete signed-catalog
archive. A client must have the identical approved package; a missing ID,
version, hash or mismatched value fails the join with a useful reason. Verify
the hash against the bytes actually loaded, not a self-reported manifest value.
Server-only packages need not be present on clients; client-only packages need
not be installed on the server, subject to server policy.

The existing mod-list comparison checks a hash mismatch only when both hashes
are present (`horizon/crates/horizon-protocol/src/mod_list.rs:194`). Therefore
strict fail-closed Wasm matching needs a versioned, negotiated protocol extension
and tests; the current comparison alone does not implement this proposal.
Servers must not download or execute a mod on a client's behalf, and a matching
hash is compatibility evidence, not remote attestation of a modified client.
Vanilla peers and connections without mods keep their existing behavior; do not
send a mandatory new handshake to them. Define a clear rejection for an older
client attempting to join a server that requires Wasm mods.

## 6. Mod SDK and developer workflow

Provide a versioned Rust guest SDK crate, generated WIT and `wit-bindgen`
bindings. Offer idiomatic wrappers for subscription, typed handles, errors,
permissions and per-side services without hiding the underlying version or
silently changing guest privileges. Runtime code shipped to players remains Rust
and Wasm; non-Rust tooling, if any, is only development tooling.

Extend the mod kit with a minimal component template, manifest examples,
capability documentation, a local validation command and compatibility fixtures.
Keep the game repository public and mod source repositories private. Developers
build each guest in their private repository, for example:

```sh
cargo build --release --target wasm32-wasip2
cargo horizon-mod pack
```

These are proposed commands, not instructions to run them in this environment.
The build and pack tools must actually produce/validate the selected component
world and place the declared side artifacts at `bin/wasm/<name>.wasm`, alongside
the package's data and assets. Extend the pack command and manifest validation;
do not assume the existing native packer already performs these steps. Ensure
reproducibility to the extent needed for catalog signing and multiplayer hashes.

`wasm32-wasip2` and the Rust standard library may introduce WASI imports. W1
must inspect the produced component's actual imports and either supply a minimal,
reviewed set of non-ambient compatibility interfaces or adjust the SDK/build;
do not accidentally grant general WASI access. Reject unexpected imports and
wrong interface versions with actionable diagnostics. Do not require mod authors
to install a signing key on players' machines.

## 7. Migration and verification

The estimates below are engineering days, not elapsed dates or commitments;
they include implementation and phase-specific tests, but exclude unknown
licensing, signing-policy, release-management and third-party review delays.
They must be refined after an inventory of each private mod and target platform.
Do not remove the existing working implementation before its replacement passes
the relevant parity gates. None of the checks below was run for this proposal.

**W1 - Runtime, first interface and example: 20-35 days.**

- Engine and launcher: embed the sandbox; generate and version an initial
  reviewed hook and service set, subscription dispatch, component loader,
  format-2 validation, signed-package verification, modded join handshake,
  permission prompts and enforcement, SDK, example `.yak` and compatibility
  fixture.
- Unit/integration: generator determinism and signature checks; wrong imports,
  ungranted calls, invalid handles, memory/fuel/deadline limits, traps, package
  integrity, cached-artifact rejection, old-world loading and no-mod baseline.
- Offscreen two-client CI: load a small both-sided example, subscribe to one
  hook, verify matching and mismatched package joins, then run a no-mod session
  without Wasm initialization. Capture timing and executable/build size.

**W2 - Yasagashi host services: 25-45 days.**

- Engine: map all 41 inventoried yasagashi hooks and required service semantics.
  Only seven are fully covered by the current 19-hook DLL subset; one more is
  partial (`reports/YK1.md:16`). Implement commands, player and entity queries
  and operations, inventory screens and rendering, effects, bossbars, chat and
  messaging, world access, map construction, block entities, sounds, persistent
  data and a capability-checked voice-range broker. Cover the remaining inventoried
  operations too (`reports/YK1.md:34`); define permissions and async behavior.
- Unit/integration: exercise every mapped hook and service, authority checks,
  lifecycle/handle invalidation, quotas, command conflicts, permissions, malformed
  input and native-versus-Wasm behavioral equivalence with matching game state.
- Offscreen two-client CI: run a representative guest against a server and two
  clients; verify commands, inventory, map/block-entity changes, sound events,
  voice-range broker behavior and denied capabilities.

**W3 - Yasagashi package and cutover: 15-30 days.**

- Mod and engine: port yasagashi to private Rust guest components and data/assets,
  package and publish a signed-catalog `.yak`, and update mod-kit instructions.
  Remove its built-in registration from dev-game only after parity is established
  (`mods/dev-game/src/main.rs:32`).
- Unit/integration: compare game rules, state changes, hooks and failure paths
  against the pre-port implementation; test package build, permissions, upgrades,
  exact hash checks and save/reload or reconnect where the mod requires them.
- Offscreen two-client CI: execute its existing representative scenarios in both
  implementations, including joins, role/game flow and disconnections; compare
  outcomes. Existing offscreen tooling provides a model, not an already-passing
  Wasm test (`mods/yasagashi/tools/e2e.py:55`).
- Voice is a dependency: yasagashi calls the Nestgate voice API
  (`mods/yasagashi/api/src/voice.rs:24`, `mods/nestgate/voice/src/public.rs:72`).
  W3 is complete only with an approved temporary native voice provider behind
  the W2 broker or after W5 provides equivalent behavior. Otherwise retain the
  built-in path and mark voice parity and cutover blocked, not complete.

**W4 - Other mods and catalog completion: 80-160 days.**

- Engine and private mods: port kagerou, replay and kigurumi in that order;
  implement their reviewed services and complete the full native hook catalog's
  typed Wasm mappings, including lifecycle and side semantics. Publish additive
  interfaces while retaining old-world shims and previously compiled fixtures.
- Unit/integration: per-mod behavioral parity, all catalog mappings and error
  fallbacks, resource lifetime, version compatibility, permissions and upgrade
  tests; regression-test native and no-mod behavior.
- Offscreen two-client CI: one representative scenario per mod and a combined
  session where supported; cover replay capture/playback, appearances, reconnect,
  incompatible package rejection and mixed native/Wasm ordering as applicable.

**W5 - Nestgate and voice: 40-80 days.**

- Engine and private mod: port the Rust Nestgate client, server-facing behavior
  and voice behind scoped interfaces. Keep device capture, codec, transport and
  device lifecycle in appropriately audited host services; guests receive only
  bounded operations and data required by the approved voice design.
- Unit/integration: device absence, consent and revocation, range and mute
  decisions, bounded queues, disconnects, errors, codec/transport behavior,
  guest isolation and compatibility with yasagashi's voice broker.
- Offscreen two-client CI: simulate or use a controlled virtual audio source,
  verify range/mute transitions and reconnect, and test explicit microphone
  denial. Hardware/device and real-network acceptance remains a separate gate.

Indicative total: 180-350 engineering days; W1-W3 total 60-110 days. Parallel
work may reduce elapsed time, not the total effort. The existing e2e scripts are
reference material; CI environments must separately provide builds and offscreen
game instances. No Cargo, game, server or test execution occurred in this task.

## 8. Risks and owner decisions

**Risks.** Full hook coverage requires representing mutable and lifetime-bound
native APIs safely, which can be more work than the hook count suggests. A
stable ABI can preserve old mistakes indefinitely. JIT security and signing,
cache authenticity, resource exhaustion, voice privacy, deterministic ordering,
server authority, package tampering and Windows device differences all need
focused reviews. WASM sandboxing does not make a permissioned host call safe.

**Owner decisions needed.**

1. Confirm Wasmtime 48 LTS/component model and acceptable executable-size,
   startup, frame/tick overhead and build-time budgets after measurement.
2. Approve permission granularity, update re-consent, administrator policy,
   developer overrides and the desired handling of a denied capability.
3. Confirm exact whole-archive hash matching, client-only mod policy, package
   distribution and whether older modded clients may join any new server.
4. Choose W3's temporary native voice provider or defer its cutover until W5;
   define acceptable voice and recording-consent UX.
5. Set supported platforms, interface-major support lifetime, resource limits,
   cache threat model and which private mods have priority if estimates change.

**Unknown / needs confirmation.** Upstream's complete rationale for removing
Wasm is not established by the inspected commits. Exact release sizes and build
times, SignPath JIT policy, complete kagerou/replay/kigurumi/Nestgate service
inventories, deployable voice-provider path and actual `wasm32-wasip2` imports
remain unverified. Resolve them before treating the schedule or W3 cutover as a
commitment.

## 9. Owner decision 2026-11-14: maximum extensibility

The owner chose breadth over conservative review: Yoake is used inside friend
groups, so "anything can be rewritten" is the goal, and the only hard line is
that a mod must not be able to damage the player's PC. This supersedes the
"a hook is unavailable until reviewed" stance of §3 and the phase scoping of §7.

- Expose the **whole native hook catalog** to WASM through the generator, with
  contracts listen / cancel / mutate / replace. Hooks whose arguments need typed
  operations get them generated from catalog metadata; nothing is hand-picked.
- Registries for blocks, items, entities, commands, screens, key bindings,
  sounds, and custom mod packets (client <-> server).
- Rendering: mods may supply WGSL shaders and draw lists; the host validates
  shaders (naga) and runs them in its own pipelines. The worst case is a GPU
  timeout, not system damage.
- Files: the mod's own data folder and world data, nothing else. Network: HTTP
  and sockets after one install-time prompt. Processes: only tools bundled with
  Yoake (e.g. ffmpeg) through a host service.
- Hard line (unchanged): no files outside the game/mod folders, no arbitrary
  processes, no native code, no raw host memory.
- Developer mode adds hot reload of `.wasm`; a scripting mod (e.g. Lua inside
  WASM) is an ordinary mod on top of this.

New phase order: W1 (in progress) → **W1x: generate all catalog hooks for both
sides** → W2 registries + rendering + files/net services → W3 yasagashi cutover
→ W4 other mods → W5 Nestgate/voice.

## 10. Owner decision 2026-11-14: Yoake 1.0 is a "complete" engine and launcher

Sign the launcher and the game once and rarely rebuild them. Yoake 1.0 is
released (signed, Store) only when: the whole hook catalog and all services
(including microphone/voice, rendering/shaders, files, network, bundled tools)
are exposed; every existing mod has been ported to WASM and passes its checks
(each mod's `WASM-NEEDS.md` is the checklist); the launcher takes all lists,
news and texts from the feed; developer features (hot reload) are built in.
Engine updates after 1.0 only for Minecraft version changes, serious bugs or
security fixes, and additive interface versions.
