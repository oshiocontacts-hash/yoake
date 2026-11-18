# Building across git worktrees

Sharing a single `CARGO_TARGET_DIR` across multiple git worktrees is unsafe.

## Why it breaks

Cargo hashes workspace members from crate metadata and build inputs, not from the worktree directory path, so the same crate gets the same artifact hash in every worktree. Cargo decides whether an artifact is fresh by comparing its modification time against source file timestamps (`mtime`).

If worktree A compiles a newer artifact into a shared `CARGO_TARGET_DIR`, worktree B can see an artifact newer than B's own checked-out source files and silently reuse A's build instead of recompiling B's sources. During MX3CS2, the first `SLOT_CAPACITY` measurement was wrong for this reason and had to be re-measured after touching the source.

## What to do instead

- **Give each worktree its own target directory** (the default `<worktree>/target`, or a worktree-specific `CARGO_TARGET_DIR`).
- **If you must run a comparison build in a shared target dir**, `touch` the changed source files in the worktree you are building immediately before running `cargo`, or run `cargo clean -p <crate>`.
- **CI** checks out one workspace per job, so it is unaffected.

## Guard in `yoake-hook-thir`

`yoake-hook-thir dump` resolves the target dir (`CARGO_TARGET_DIR` if set, otherwise `<workspace>/target`) and checks `<target>/.yoake-workspace-root`, which records the canonical workspace root that owns that target directory:

- missing marker: writes the current canonical workspace root;
- same root: proceeds;
- different root: refuses with an error explaining the stale-artifact hazard.

Pass `--allow-shared-target` only when intentionally sharing a target directory after ensuring freshness.

## The pinned toolchain

`rust-toolchain.toml` at the workspace root pins rustc **1.100.0** (with rustfmt, clippy, rust-src and the
`wasm32-unknown-unknown` / `wasm32-wasip2` targets). rustup picks it up for every cargo and rustc call in the
workspace; CI installs it with `rustup toolchain install` (no argument: it reads the file). `yoake-bindgen` (the CLI
and the regeneration gate) and `yoake-hook-thir dump` run `rustc -V` in the workspace and stop when it is not the
pinned version, because the rustdoc JSON and the THIR text they read change with the compiler.

To move to another compiler: change `channel` in one commit together with the regenerated glue
(`crates/yoake-bindgen/regen.sh`), the bindgen gate passing, and a THIR verify run (`verify-callsites`) on it.

A machine that already has that exact version installed under another name (for example as `stable`) can use it
without a download through a rustup directory override (`rustup override set stable` in the worktree) or
`RUSTUP_TOOLCHAIN`: both take precedence over the file, and the version check still guards the result.
