# `firmware`

Targets the ESP32-S3's Xtensa core (`xtensa-esp32s3-none-elf`), which needs
the esp-rs fork of rustc+LLVM (installed via `espup`) — mainline Rust's
Xtensa target *spec* is upstream, but Xtensa codegen isn't.

**This crate cannot be built in a Claude Code Remote session.** `espup`
downloads its toolchain from `objects.githubusercontent.com` and
`dl.espressif.com`, both blocked by this environment's egress policy
(confirmed by testing — see the design brief's setup discussion). It's
deliberately left out of the workspace's `default-members` so routine
`cargo check`/`clippy`/`test` runs don't fail on it.

It's compiled in CI instead (`.github/workflows/ci.yml`, `firmware` job),
using `esp-rs/xtensa-toolchain` on a GitHub-hosted runner, which isn't
behind this proxy. To build it locally on your own machine: install
`espup` (https://github.com/esp-rs/espup), run `espup install`, source the
generated export file, then `cargo check -p firmware --target
xtensa-esp32s3-none-elf`.

`panel-backend` has the same constraint and is excluded from
`default-members` for the same reason.
