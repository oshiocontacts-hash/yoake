# 1. Setting Up Your Environment

Yoake mods are written in the **Rust** programming language and compiled (built) into **WebAssembly (`wasm32-wasip2`)**.
This page walks you through installing the development tools on your computer, focusing on Windows with brief notes for macOS and Linux where needed.

---

## 1-1. Install Rust (rustup)

Start by installing **rustup**, the official Rust toolchain installer. To match the crates that the CLI in this guide depends on, make sure you have **Rust 1.95 or newer**.[^rust-version]

### On Windows

1. Open the official site (`https://rustup.rs/`) in your browser, download `rustup-init.exe`, and run it.
2. Follow the on-screen prompts to complete the installation (if prompted to install the Visual Studio C++ Build Tools, follow the instructions to install them as well).
3. Once the installation finishes, open a new **PowerShell** or **Command Prompt** window.

### On macOS / Linux

- **macOS**: Open Terminal and install `rustup` using the command on the official site (`https://rustup.rs/`). If Xcode Command Line Tools are not installed yet, they will be installed along the way.
- **Linux**: Open your distribution's terminal and follow the instructions on the official site (`https://rustup.rs/`) to install `rustup`.

---

## 1-2. Add the WebAssembly Target (`wasm32-wasip2`)

By default, Rust compiles programs for the computer you are currently using. Yoake mods, however, are built as **WebAssembly components (`wasm32-wasip2`)** that run inside the game's sandbox.

Open PowerShell (or Terminal on macOS / Linux) and run this command once:

```sh
rustup target add wasm32-wasip2
```

- That is all you need to output WebAssembly components.
- **Note**: You do not need to install external tools like `wasm-tools` separately.

When setting an environment variable in PowerShell, use this syntax:

```powershell
$env:NAME='value'
```

This setting only applies to **that single PowerShell window** and disappears as soon as you close the window.

---

## 1-3. Install the `yoake` CLI

The `yoake` command-line tool (used in the next chapter for `yoake new`, `yoake build`, and so on) lives in this repository at `crates/yoake-cli` (package name `yoake`, Rust 1.95 or newer).[^cli] For now, you install it from the public repository. From the repository's root folder, run:

```sh
cargo install --path crates/yoake-cli
```

Cargo puts the built `yoake` executable in its `bin` folder (`~/.cargo/bin`), which `rustup` already adds to your `PATH`. After installing, open a new terminal and check that `yoake` runs.

1.0 is planned to also put a Windows executable on GitHub Releases (undecided).

---

## 1-4. Prepare a Text Editor

Mod source files (`.rs` files) and configuration files (`yak.toml`, `Cargo.toml`) are plain text files, so any text editor you like will work.

If you aren't sure which one to pick, **Visual Studio Code (VS Code)** is a great choice:

- Installing the **rust-analyzer** extension gives you real-time autocompletion for function and hook names and highlights mistakes as you type, making Rust much easier to write even for first-timers.
- A TOML extension (such as Even Better TOML) is also handy for reading and editing `yak.toml` and `Cargo.toml`.

---

## 1-5. Choose a Workspace Folder

Create a dedicated workspace folder where you'll keep your mod projects.

- On Windows, pick a simple path without spaces or non-ASCII characters—for example, `C:\Users\<your-username>\yoake-mods-work`—to avoid path-related hiccups.
- On macOS and Linux, create a folder such as `~/yoake-mods-work` under your home directory.
- Each mod gets its own subfolder. When you run `yoake new` in the next chapter, it will generate a fresh folder for your new mod inside this workspace folder.

> [!NOTE]
> On Windows, names like `con`, `prn`, `aux`, `nul`, `com1`–`com9`, and `lpt1`–`lpt9` are reserved by the operating system and cannot be used as mod names or folder names (`yoake new` will also reject them). Stick to lowercase ASCII letters, digits, and hyphens (`-`), such as `my-hud`.

---

## Next Steps

Once your environment is ready, head over to [2. Creating Your First Mod (`02-first-mod.md`)](./02-first-mod.md) to create and build your first HUD mod.

[^rust-version]: The shared workspace `rust-version` is 1.85, while `horizon-mods` (used by the CLI) and the CLI itself set `rust-version` to 1.95. Source: `Cargo.toml`, `crates/horizon-mods/Cargo.toml`, and `crates/yoake-cli/Cargo.toml`.
[^cli]: Source: `crates/yoake-cli/Cargo.toml` (`name = "yoake"`).
