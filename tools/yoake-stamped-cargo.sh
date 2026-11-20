#!/usr/bin/env bash
# Runs cargo with the Yoake hook stamps applied at build time (docs/yoake2/reports/FR-F1-W2.md).
#   tools/yoake-stamped-cargo.sh build --profile fast -p horizon-client --features yoake2-slice
# The repository stays unstamped; the stamped overlay lives in $YOAKE_STAMP_TARGET/yoake-stamp (default
# target-stamped/). A plain `cargo build` never stamps: this wrapper is the only thing that turns it on.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
tools_target="${YOAKE_TOOLS_TARGET:-$root/target-tools}"
CARGO_TARGET_DIR="$tools_target" cargo build --offline --release -p yoake-hook-codemod >&2
exe="$tools_target/release/yoake-hook-codemod"
[ -x "$exe" ] || exe="$exe.exe"
export YOAKE_STAMP=1
export YOAKE_WORKSPACE="$root"
export RUSTC_WORKSPACE_WRAPPER="$exe"
export CARGO_TARGET_DIR="${YOAKE_STAMP_TARGET:-$root/target-stamped}"
export YOAKE_STAMP_DIR="$CARGO_TARGET_DIR/yoake-stamp"
exec cargo "$@"
