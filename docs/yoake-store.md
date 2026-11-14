# Yoake Microsoft Store package

## Build

The `yoake-windows` GitHub Actions workflow first builds the normal Windows artifacts, then runs a
separate Windows job that rebuilds the plain game and launcher with `YOAKE_DISTRIBUTION=store`.
The job creates an unsigned `Yoake-<version>-x64.msix` using the Windows SDK and uploads it as the
`yoake-msix` artifact. Microsoft signs the package through the Store submission process; this
artifact is not a signed, sideload-ready release.

Set the normal Yoake build variables used by the Windows workflow, plus these repository variables
from **Partner Center → Product identity**:

| Repository variable | Partner Center value |
|---|---|
| `YOAKE_MSIX_IDENTITY_NAME` | Package identity name |
| `YOAKE_MSIX_PUBLISHER` | Publisher distinguished name |
| `YOAKE_MSIX_PUBLISHER_DISPLAY_NAME` | Publisher display name |

The MSIX job fails if any of those three are missing. `YOAKE_VERSION` must be three numeric
components, each at most 65535; for example, `1.2.3` becomes MSIX version `1.2.3.0`.
Use the exact identity values assigned to the Store product.

## Submit

Download the `yoake-msix` workflow artifact and upload the MSIX in **Partner Center → submission →
Packages**. Configure the intended hidden, link-only Store availability in Partner Center and give
the listing a URL for the [privacy policy](yoake-privacy.md). Invite and beta-code access remain
separate from Store availability.

## Store mode

- The launcher does not self-update; the Microsoft Store updates the package instead.
- The package includes `yoake.exe` beside `YoakeLauncher.exe`. A plain Windows engine with the
  bundled version is used only when the signed feed also identifies exactly that file by size and
  SHA-256. It is copied into the existing verified object cache for installation and launched from
  the package after verification. No verification is bypassed.
- Other versions, bases with compiled-in native mods, or a non-matching bundled executable use the
  existing download and installed-engine path; the launcher records fallbacks in
  `logs/yoake-store.log` under its data directory when writable. Such an executable may still be
  unsigned and blocked by Smart App Control.
- Launcher-managed state and the game's working directory remain in their existing user data
  locations. A CLI export without an explicit destination defaults to the launcher data folder in
  Store mode, not the working folder.

Before submitting, verify that the packaged `yoake.exe` matches the signed feed's plain-engine hash
and size for the same version. A separately built binary that differs will safely fall back to the
download path even if its version matches. The Store owns package updates; a newer package version
must be submitted through Partner Center.
