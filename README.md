# Journal

A native, offline plain-text journal for Linux, Windows, and macOS, built with Rust, eframe, and SQLite through SQLCipher.

## Run

Use current stable Rust and Cargo. Building the bundled SQLCipher and OpenSSL requires a C toolchain, Perl, and make (MSVC tools and NASM on Windows, Xcode command-line tools on macOS). On Debian-family desktops install build-essential, perl, pkg-config, libxkbcommon-dev, libwayland-dev, libgl1-mesa-dev, and libx11-dev.

```sh
cargo run --release
```

Choose encrypted or plain storage on first launch. Encrypted storage requires entering and confirming a nonempty passphrase. Closing setup before choosing Create leaves the database path available. Existing encrypted journals prompt for the passphrase. There is no password reset or recovery key.

Click **New entry** to write. Changes save after 750 ms of inactivity, when switching entries, and on normal window close. The sidebar uses creation time and displays local dates. An abrupt process exit can lose edits still waiting for autosave.

## Storage

The app opens journal.db in the per-user local application data directory selected by the directories crate. On a typical Linux installation this is ~/.local/share/journal/journal.db. Journal content and timestamps share the same database. The app uses SQLCipher's default encrypted format and key derivation for password-protected databases. It does not store the passphrase. The plain storage option writes an ordinary SQLite database.

The version-one schema lives in src/schema.sql. Database initialization occurs in a temporary file in the destination directory. The completed file is published only if the destination is still available. An existing file with an unknown header is treated as potentially encrypted. Failed reads never trigger automatic recreation.
