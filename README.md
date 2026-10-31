<div align="center">

# Reo Launcher

A Minecraft: Java Edition launcher with packaged, pre-configured mods in one click.

</div>

## Installing

You can install the latest release from our [GitHub releases](https://github.com/drako1k/reo-launcher/releases/latest).

| Windows (x86_64) | macOS (Intel & Apple Silicon) | Linux (x86_64) |
|------------------|-------------------------------|----------------|
| Installer 🔄      | DMG 🔄                        | AppImage 🔄     |
|                  | App Bundle 🔄                 | DEB            |
|                  |                               | RPM            |

> 🔄 = Has support for autoupdating built-in


## Contributing

We welcome contributions! Please read our [contributing guidelines](CONTRIBUTING.md) before getting started.


### Requirements

The project targets **Rust 1.97** or later. You can install Rust via [rustup](https://rustup.rs/).


### Building & Running

```sh
# Run the app
cargo run -p reo_app

# Build a release binary
cargo build -p reo_app --release
```


### Packaging / Releasing

Installers are produced with [**cargo-packager**](https://github.com/crabnebula-dev/cargo-packager).
Config lives in [`packages/reo_app/Cargo.toml`](./packages/reo_app/Cargo.toml) under
`[package.metadata.packager]`.

```sh
cargo install cargo-packager --locked

# Build the binary, then bundle it for the current OS:
cargo build --release -p reo_app
cargo packager --release -p reo_app --formats <targets>
#   Windows: nsis      macOS: app,dmg      Linux: deb,appimage
```


### Versioning

The workspace shares a single version, defined in the root [`Cargo.toml`](./Cargo.toml) under `[workspace.package]`.

Versions and release notes come from [Knope](https://knope.tech) change files in [`.changeset/`](./.changeset).
Add one per user-facing change with `knope document-change` (installed via `cargo install knope`). The
`Reo Launcher Release Build` workflow consumes them, bumps the version, writes [`CHANGELOG.md`](./CHANGELOG.md),
and uses the new entry as the GitHub release body, which is what the launcher's changelog page shows.


## Acknowledgements

Reo Launcher includes code from [OneLauncher](https://github.com/Polyfrost/OneLauncher) by Polyfrost and its contributors, used under its original license (see [LICENSE](./LICENSE)).
