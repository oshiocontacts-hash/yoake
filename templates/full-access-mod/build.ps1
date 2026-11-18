# Builds the mod to a WebAssembly core module and packs it into dist\<id>-<version>.yak.
#   $env:YOAKE_REPO       the Yoake source checkout (default: two folders up from this one)
#   $env:YOAKE_TOOLCHAIN  the rustc toolchain (default: 1.100.0, the version the checkout pins)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$repo = if ($env:YOAKE_REPO) { $env:YOAKE_REPO } else { (Resolve-Path (Join-Path $here '..\..')).Path }
$tc = if ($env:YOAKE_TOOLCHAIN) { $env:YOAKE_TOOLCHAIN } else { '1.100.0' }
$sdk = Join-Path $repo 'crates\yoake-guest-sdk\src\lib.rs'
if (-not (Test-Path $sdk)) { throw "no SDK at $sdk; set YOAKE_REPO to your Yoake checkout" }

# the module name must match [wasm] server in yak.toml: bin/wasm/<name>.wasm
$name = 'my_full_access_mod'
$common = @('--edition', '2024', '--target', 'wasm32-wasip2', '-C', 'panic=abort', '-C', 'opt-level=1')

function Run($exe, $arguments) {
    & $exe @arguments
    if ($LASTEXITCODE -ne 0) { throw "$exe failed with exit code $LASTEXITCODE" }
}

$build = Join-Path $here 'build'
if (Test-Path $build) { Remove-Item -Recurse -Force $build }
New-Item -ItemType Directory -Force (Join-Path $build 'sdk'), (Join-Path $here 'bin\wasm'), (Join-Path $here 'dist') | Out-Null

Write-Host '1/3 SDK (rlib)'
Run 'rustc' (@("+$tc") + $common + @('--crate-type', 'rlib', '--crate-name', 'yoake_guest_sdk', '--out-dir', (Join-Path $build 'sdk'), $sdk))

Write-Host '2/3 mod (core module)'
$rlib = Join-Path $build 'sdk\libyoake_guest_sdk.rlib'
Run 'rustc' (@("+$tc") + $common + @(
    '--crate-type', 'cdylib', '--crate-name', $name,
    '-C', 'linker=rust-lld', '-C', 'linker-flavor=wasm-ld', '-C', 'strip=symbols',
    '-C', 'link-arg=--no-entry', '-C', 'link-arg=--export-memory',
    '-C', 'link-arg=--export=yoake_hook', '-C', 'link-arg=--export=yoake_init',
    '-L', "dependency=$(Join-Path $build 'sdk')", '--extern', "yoake_guest_sdk=$rlib",
    '-o', (Join-Path $here "bin\wasm\$name.wasm"), (Join-Path $here 'src\lib.rs')))

Write-Host '3/3 pack'
# `cargo-horizon-mod pack` writes <id>-<version>.yak into the folder; it refuses to overwrite one.
Get-ChildItem -Path $here -Filter '*.yak' | Remove-Item -Force
Run 'cargo' @("+$tc", 'run', '-q', '-j', '4', '--manifest-path', (Join-Path $repo 'Cargo.toml'), '-p', 'cargo-horizon-mod', '--', 'pack', $here)
Get-ChildItem -Path $here -Filter '*.yak' | Move-Item -Destination (Join-Path $here 'dist') -Force
Get-ChildItem (Join-Path $here 'dist') -Filter '*.yak' | Format-Table Name, Length
