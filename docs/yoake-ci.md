# Yoake Windows CI

The `yoake-windows` workflow builds the release launcher and the plain game client
without compiled-in workspace mods. It uploads one unsigned artifact,
`yoake-windows-unsigned`, containing `YoakeLauncher.exe`, `yoake.exe`, and
`SHA256SUMS`. Signing is not configured yet.

## Repository variables

Ask the owner for the values of these public repository variables:

- `YOAKE_VERSION`
- `YOAKE_FEED_URL`
- `YOAKE_PORTAL_URL`
- `YOAKE_FEED_SOURCE_ID`
- `HORIZON_MSA_CLIENT_ID`
- `YOAKE_PUBLIC_KEYS` (public verification key line or lines)

The workflow requires every variable and uses the public key lines for both
builds. Signing keys must never be put in CI; the owner handles signing.

## Run the workflow

In GitHub, go to **Actions → yoake-windows → Run workflow**. A push of a tag
matching `yoake-v*` also runs it.
