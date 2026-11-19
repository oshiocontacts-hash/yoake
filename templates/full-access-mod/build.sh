#!/bin/sh
# Builds the mod to a WebAssembly core module and packs it into dist/<id>-<version>.yak.
#   YOAKE_REPO       the Yoake source checkout (default: two folders up from this one)
#   YOAKE_TOOLCHAIN  the rustc toolchain (default: 1.100.0, the version the checkout pins)
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=${YOAKE_REPO:-$(cd "$HERE/../.." && pwd)}
TC=${YOAKE_TOOLCHAIN:-1.100.0}
SDK="$REPO/crates/yoake-guest-sdk/src/lib.rs"
[ -f "$SDK" ] || { echo "no SDK at $SDK; set YOAKE_REPO to your Yoake checkout" >&2; exit 1; }

# the module name must match [wasm] server in yak.toml: bin/wasm/<name>.wasm
NAME=my_full_access_mod
COMMON="--edition 2024 --target wasm32-wasip2 -C panic=abort -C opt-level=1"

rm -rf "$HERE/build"
mkdir -p "$HERE/build/sdk" "$HERE/bin/wasm" "$HERE/dist"

echo "1/3 SDK (rlib)"
rustc +"$TC" $COMMON --crate-type rlib --crate-name yoake_guest_sdk --out-dir "$HERE/build/sdk" "$SDK"

echo "2/3 mod (core module)"
rustc +"$TC" $COMMON --crate-type cdylib --crate-name "$NAME" \
  -C linker=rust-lld -C linker-flavor=wasm-ld -C strip=symbols \
  -C link-arg=--no-entry -C link-arg=--export-memory \
  -C link-arg=--export=yoake_hook -C link-arg=--export=yoake_init \
  -L "dependency=$HERE/build/sdk" --extern "yoake_guest_sdk=$HERE/build/sdk/libyoake_guest_sdk.rlib" \
  -o "$HERE/bin/wasm/$NAME.wasm" "$HERE/src/lib.rs"

echo "3/3 pack"
# `cargo-horizon-mod pack` writes <id>-<version>.yak into the folder; it refuses to overwrite one.
rm -f "$HERE"/*.yak
cargo +"$TC" run -q -j 4 --manifest-path "$REPO/Cargo.toml" -p cargo-horizon-mod -- pack "$HERE"
mv "$HERE"/*.yak "$HERE/dist/"
ls -l "$HERE"/dist/*.yak
