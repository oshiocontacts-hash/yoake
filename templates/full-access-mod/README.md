# Full-access mod starter

日本語: [README.ja.md](README.ja.md)

A minimal Yoake 2.0 full-access mod. On every server tick it makes two engine calls (it opens the `Game.experience`
handle and reads `Experience.total`) and then lets the tick run. Read
[`docs/yoake2/MOD-GUIDE.md`](../../docs/yoake2/MOD-GUIDE.md) for what a full-access mod is and what the player has to do.

## Files

| file | what it is |
|---|---|
| `yak.toml` | the package manifest (`world = "full-access"`, one server module) |
| `src/lib.rs` | the mod: `init` registers `hooks::SERVER_TICK`, `hook` makes the engine calls and calls `original` |
| `build.sh`, `build.ps1` | build the SDK and the mod with `rustc`, then pack `dist/<id>-<version>.yak` |

## Build

You need the Yoake source checkout (this folder lives in it, at `templates/full-access-mod`) and rustc **1.100.0**
with the `wasm32-wasip2` target (the checkout's `rust-toolchain.toml` pins both).

```sh
./build.sh          # Linux, macOS, Git Bash
```

```powershell
.\build.ps1         # Windows PowerShell
```

Both scripts do three steps: compile `crates/yoake-guest-sdk` for `wasm32-wasip2` as an rlib, compile `src/lib.rs`
against it into `bin/wasm/my_full_access_mod.wasm` (a core module, no component wrapper), and pack it with the
checkout's own tool, `cargo run -p cargo-horizon-mod -- pack`. The result is `dist/my_full_access_mod-0.1.0.yak`
(unsigned: a local install does not need a signature).

Settings: `YOAKE_REPO` (the checkout; default two folders up), `YOAKE_TOOLCHAIN` (default `1.100.0`).

If you copy the template out of the checkout, set `YOAKE_REPO` to the checkout.

On Windows with the GNU toolchain, compiling the pack tool needs a MinGW `bin` folder (with `dlltool` and `as`) on
`PATH`. The mod itself needs only `rustc`.

## Make it your own

1. Pick an id (3 to 48 characters of `a-z 0-9 _ -`) and change it in `yak.toml` (`id`, and the file name in
   `[wasm] server`), in `NAME` in both scripts, and in `build.ps1`'s `$name`. The module file must be directly under
   `bin/wasm/` and end in `.wasm`.
2. Change `name`, `authors`, `description`, and bump `version` for every release (the player allows each new file again).
3. Write your hooks in `src/lib.rs`.

## Install

In the launcher: Mods page, **Add a .yak file…** (or drag and drop), then switch the mod to **Allowed** and confirm.
The launcher records the exact bytes; a rebuilt file has to be allowed again. Start the game from the launcher
afterwards.

## Test without the game

`crates/horizon-client/tests/yoake2_template.rs` builds this template, packs, installs, approves and loads it through
the real loader and ticks it on a stand-in `Game`:

```sh
cargo test --offline -j 4 -p horizon-client --features yoake2-slice,yoake-hook-bench --test yoake2_template
```

`report(slot, value)` in `src/lib.rs` is a test aid: only tests read it, so the template calls it on the first three
ticks only.
