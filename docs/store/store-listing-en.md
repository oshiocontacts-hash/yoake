## App Name

Yoake

## Short Description

A Rust game client and launcher: explore worlds, multiplayer with friends, and mods in one app.

## Description

Yoake is a Rust game client and launcher compatible with Minecraft: Java Edition 26.3. Enjoy familiar gameplay—world generation, mobs, redstone, commands, and the recipe book—along with your existing saves and multiplayer. Vanilla saves work out of the box, and worlds from 1.8 through 26.2 upgrade smoothly to 26.3. Prepare and update the game, manage instances, and install mods all from a single launcher. Because each instance keeps its own game version and mod setup, you can easily separate the setup you use with friends from your solo worlds. On Windows, the Microsoft Store edition handles installation and updates for you automatically.

Standard play requires signing in with a Microsoft account that owns Minecraft: Java Edition. Yoake can reuse an existing game installation on your PC, or download and prepare the required game files from official distribution sources for your account if none is installed. Yoake is free and open-source software and does not bundle any code or assets from the original game.

This app is not an official Minecraft product and is not approved by or associated with Microsoft.

Open your single-player world directly to friends, or join LAN games and online servers. To play with friends, the host generates an invite code and guests enter it in the game or launcher to join. Traffic between the host and joining players is encrypted. Depending on your network environment, port forwarding, a VPN, or a self-hosted relay server may be required. Invite codes expire as soon as you stop sharing or close the world.

The launcher's Friends page lets you send and receive friend requests via friend codes, create groups and invite members, and manage blocked users. It requires signing in to the Yoake portal and displays "Friends are not available yet." until enabled on the portal. This social feature is separate from world invite codes—even when Friends is unavailable, playing via invite codes, LAN, or servers works as usual.

Mods are built primarily as WebAssembly (Wasm) .yak packages. Wasm mods run inside an in-game sandbox with declared permissions and execution-time and memory limits. Running mods that contain code in a normal environment requires a valid Yoake signature; unsigned mods you build yourself can only be tested under developer settings. Java mod JAR files do not run.

For mod development, Yoake provides a Rust SDK, examples, hot reload, and in-engine testing. Mods can add HUD elements, custom commands, settings, and data storage. You can save your chosen base pack and mods as a configuration and share it with a YK1 share code to import as a new instance. Available mods depend on what is listed in the catalog.

Modpacks from Modrinth and CurseForge can be imported, bringing over the data and asset content that Yoake supports. Resource packs, data packs, and shader packs are also supported. Extra settings—including graphics presets, distant terrain, and controller support—are available and default to off wherever they would alter the original look.

From the launcher, you can also create, start, back up, and update servers, and invite friends over the internet using Tailscale. The launcher interface is available in Japanese and English, with dark, light, or system appearance options and a choice of accent colors. The Microsoft Store edition of the launcher does not self-update; package updates are handled by the Microsoft Store.

Cosmetics such as hats and pets are in preparation and are currently disabled.

Compatibility is based on verified areas, and some differences from the original game remain. Not every feature or Java mod behaves identically.

## Key Features

- **Rust game client**: Supports 26.3 worlds, mobs, villagers, raids, redstone, commands, the recipe book, and existing saves.
- **Multiplayer**: Join online servers, LAN games, and friend worlds via invite codes, with encrypted friend-to-friend connections.
- **Friends and groups**: Friend codes, requests, group invites, and blocking. Requires signing in to the Yoake portal and is unavailable until enabled on the portal.
- **Wasm mods and .yak**: Sandboxed Wasm mods, a signed catalog, package verification, and client- and server-side mod support.
- **Save and share mod configurations**: Save a base pack and mod selection as a configuration and import it as a new instance from a YK1 code.
- **Packs**: Resource packs, data packs, shader packs, and Modpack imports for supported content.
- **Extra settings**: Graphics presets, distant terrain, and controller support, with anything that changes the original look off by default.
- **Instance and version management**: Multiple instances, game asset preparation, and version selection and updates.
- **Server management**: Create, start, back up, and update servers from the launcher, with support for Tailscale invites.
- **Japanese and English launcher**: Choose dark, light, or system appearance and custom accent colors.

## What's New

- **Wasm mods**: Build Wasm components from Rust and install them as .yak packages, with support for client- and server-side hooks, settings, data storage, development hot reload, and in-engine testing.
- **Friends and groups**: Added friend codes and requests, group creation and invites, and user blocking, available when enabled on the portal.
- **Mod configuration sharing**: Share combinations of a base pack and mods using YK1 share codes.
- **New launcher look**: Supports dark, light, and system appearance modes along with custom accent colors.

## Search Keywords

Yoake, Rust, game client, launcher, multiplayer, mods, Java Edition compatible

## Notes for Age Rating Questions

- **Chat, friends, and groups**: Includes in-game chat and supports joining online servers, LAN games, and friend worlds. The launcher includes friend requests via friend codes, group creation and invites, and blocking; using them requires signing in to the Yoake portal and having the feature enabled on the portal (shows "Friends are not available yet." while disabled). (Sources: `README.md`, `docs/play-with-friends.md`, `launcher/src/friends_view.rs`, `launcher/lang/ja.json`)
- **Sharing user-generated content**: Mods can be distributed and installed as .yak packages, and base pack + mod configurations can be shared with others via YK1 share codes. Cosmetics are currently off by default (`ON_BY_DEFAULT=false`); only when enabled will equipped items be shown to other compatible clients through the cosmetics service. There is currently no feature for users to upload new cosmetics; it is only planned. (Sources: `docs/api/yoake-mods.md`, `docs/yoake-configs.md`, `docs/cosmetics.md`, `crates/horizon-cosmetics/src/lib.rs`)
- **In-app purchases**: None. The reference documents describe no in-app purchases or paid item sales, the founder set for cosmetics (in preparation) is free, and there are no randomized loot boxes or gacha mechanics. (Sources: `docs/cosmetics.md`, `docs/yoake-store.md`)
- **Location and personal data**: No collection of location data is described. Signing in with a Microsoft account uses Microsoft, Xbox Live, and Minecraft services, while signing in to the Yoake portal uses an invite code or Discord. In multiplayer, game data is sent to the server or host you choose to join, and connection peers can see your IP address. Invite codes for friend worlds contain the host's LAN, VPN, and public addresses, and the STUN servers used to discover the external address see the host's public IP address. (Sources: `docs/yoake-privacy.md`, `docs/play-with-friends.md`)
- **Violence**: Follows the content of the original game, including monsters and other mobs as well as health and hunger mechanics. (Source: `README.md`)

## Privacy Highlights

- **What Yoake does not send**: Yoake builds do not send usage telemetry or crash reports. Upstream telemetry (including the anonymous "playing" ping) is disabled under the Yoake brand. Settings, world saves, installed files, and caches are stored in the user's local data folders.
- **Sign-in traffic**: Signing in with a Microsoft account communicates with Microsoft, Xbox Live, and Minecraft services for authentication required to play. Opening and signing in to the Yoake portal uses an invite code or Discord.
- **Update traffic**: The standard edition of the launcher reads version and signature metadata from the update feed at `portal.yunipop.com` and downloads the files chosen by the user. In the Microsoft Store edition, the Microsoft Store installs and updates the package according to the Store's own settings.
- **Multiplayer traffic**: Connects to the servers, LAN games, or friend worlds the user chooses to join and may send game data to those services. For traffic involved in friend features, see the age rating notes above.
- **How to stop data collection**: Because usage data and crash reports are not sent, no opt-out toggle is needed. The performance statistics consent screen present in the shared launcher codebase is not shown under the Yoake brand. All communication with external services is initiated by user action, so no such traffic occurs unless you use sign-in, the portal, or multiplayer. Updates for the Store edition are managed in the Microsoft Store settings.

## Sources

※ This section is an internal note to be removed before publishing.

- **App Name**: `README.md`, `launcher/lang/ja.json`.
- **Short Description**: `README.md`.
- **Description**: Compatible version, world upgrades, account ownership requirement, no bundled game code or assets, free OSS, and default values for extra settings from `README.md`; Store edition updates from `docs/yoake-store.md`; friend worlds and encryption from `docs/play-with-friends.md`; configurations and YK1 from `docs/yoake-configs.md`; mod signing, sandboxing, and developer settings from `docs/api/yoake-mods.md`, `.lane/ref/yoake-wasm-dev.md`, and `.lane/ref/guides/guide-publish/docs/guide-ja/03-play.md`; mod development from `.lane/ref/guides/*/docs/guide-ja/`; Tailscale and server management from `README.md` and `launcher/lang/ja.json`; language options from `launcher/src/i18n.rs` and `launcher/src/language_page.rs`; the "Friends are not available yet." notice from `launcher/src/friends_view.rs` and `launcher/lang/ja.json`; cosmetics from `docs/cosmetics.md` and `crates/horizon-cosmetics/src/lib.rs` (`ON_BY_DEFAULT=false`).
- **Key Features / What's New**: Same as above. Cosmetics and the wardrobe are in preparation and therefore omitted from the feature lists. Minimap and zoom are native mods (for advanced users) and are omitted because they cannot be confirmed as standard features in Yoake.
- **Search Keywords**: Based on `README.md` and the description in this document.
- **Notes for Age Rating Questions**: `README.md`, `docs/cosmetics.md`, `docs/play-with-friends.md`, `docs/yoake-privacy.md`, `docs/yoake-store.md`, `docs/yoake-configs.md`, `docs/api/yoake-mods.md`, `launcher/src/friends_view.rs`, `launcher/lang/ja.json`.
- **Privacy Highlights**: `docs/yoake-privacy.md`, `docs/yoake-signing-policy.md` (`portal.yunipop.com`), `launcher/src/telemetry.rs` (no telemetry destination in Yoake), `launcher/src/collection.rs` (consent screen not shown in Yoake). Distinction between update methods from `docs/yoake-store.md`.
