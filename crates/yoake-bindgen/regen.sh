#!/usr/bin/env bash
# Regenerates the engine glue (MX3G1): rustdoc JSON of the engine crates with the feature OFF, then the
# generator. Extra arguments go to the generator (`--check` compares instead of writing).
#
#   crates/yoake-bindgen/regen.sh            # write glue, id lock and reports
#   crates/yoake-bindgen/regen.sh --check    # exit 1 if anything would change
set -euo pipefail
cd "$(dirname "$0")/../.."
# RUSTDOCFLAGS is empty as in the gate (tests/engine.rs): the cfg of the JSON is pinned
export RUSTC_BOOTSTRAP=1 CARGO_INCREMENTAL=0 RUSTDOCFLAGS=
for krate in horizon-client-lib horizon-client horizon-items horizon-core horizon-world horizon-client-core; do
  cargo rustdoc --offline -j 4 -p "$krate" --lib -- -Z unstable-options --output-format json --document-private-items
done
cargo run --offline --release -j 4 -p yoake-bindgen -- --config crates/yoake-bindgen/bindgen.json "$@"
