# Running the server on a Raspberry Pi

The content server (`crates/server`) is a single self-contained binary: the
preview pages and the `web_preview.wasm` module are embedded in it, and it is
statically linked (musl), so it doesn't depend on the Pi's libc version. It
runs as a systemd service that starts on boot.

## 1. Get the binary

CI's `server-pi` job builds the binary for 64-bit Raspberry Pi OS (Pi 3/4/5,
Zero 2 W) and uploads it as the `server-aarch64` workflow artifact, alongside
this README and the unit file. Check that `uname -m` on the Pi prints
`aarch64`; a 32-bit OS install isn't supported.

Download it from the run's **Summary** page on GitHub (Actions → CI → a run →
Artifacts), or with the GitHub CLI:

```sh
gh run download --name server-aarch64 --dir server-aarch64   # latest run on the current branch
# or a specific run:  gh run download <run-id> --name server-aarch64 --dir server-aarch64
```

Artifacts are kept for 90 days. To build locally instead (from the repo root,
needs `rustup`; no cross C toolchain required):

```sh
scripts/build-web-preview.sh
rustup target add aarch64-unknown-linux-musl
cargo build -p server --release --target aarch64-unknown-linux-musl \
  --config 'target.aarch64-unknown-linux-musl.linker="rust-lld"' \
  --config 'profile.release.debug=false' --config 'profile.release.strip=true'
# -> target/aarch64-unknown-linux-musl/release/server
```

## 2. Install it on the Pi

Copy the binary and unit file across (replace `pi@raspberrypi.local`):

```sh
scp server-aarch64/reterminal-server server-aarch64/reterminal-server.service pi@raspberrypi.local:
```

Then on the Pi:

```sh
# Artifact zips drop the executable bit, so set the mode explicitly.
sudo install -m 0755 reterminal-server /usr/local/bin/reterminal-server
sudo install -m 0644 reterminal-server.service /etc/systemd/system/reterminal-server.service

sudo systemctl daemon-reload
sudo systemctl enable --now reterminal-server   # start now and on every boot
```

Check that it's running:

```sh
systemctl status reterminal-server
journalctl -u reterminal-server -f              # follow the logs
curl -i http://localhost:8080/screen            # 200 with the current screen JSON
```

## 3. Point the device at it

Push a template from any machine on the LAN:

```sh
curl -X PUT --data-binary @crates/screen-spec/samples/meals.json \
  http://raspberrypi.local:8080/screen
```

Open `http://raspberrypi.local:8080/` in a browser for the preview. Build the
firmware with `SCREEN_URL=http://<pi-ip>:8080/screen` (see
`crates/firmware/src/config.rs`). Use the Pi's IP address, or a hostname the
device can resolve. A DHCP reservation for the Pi stops the address from
changing.

## What the unit does

[`reterminal-server.service`](reterminal-server.service):

- Listens on `0.0.0.0:8080`. To change the port, edit `--bind` in `ExecStart`.
- Persists the template and values in `/var/lib/reterminal-server/`
  (`screen.json` and `values.json`), so they survive restarts and reboots.
- Runs as an unprivileged throwaway user (`DynamicUser=yes`). systemd creates
  and owns the state directory, so you don't need to create a user. With a
  dynamic user the files actually live in `/var/lib/private/reterminal-server/`,
  and `/var/lib/reterminal-server` is a symlink to that directory.
- Waits for `network-online.target` and restarts on failure.
- Stops with `SIGINT`, which the server handles as a graceful shutdown.
- Sets log verbosity through `RUST_LOG`. To change it without editing the
  file, run `sudo systemctl edit reterminal-server` and add
  `[Service]` / `Environment=RUST_LOG=debug`.

## Upgrading

```sh
sudo install -m 0755 reterminal-server /usr/local/bin/reterminal-server
sudo systemctl restart reterminal-server
```

The upgrade leaves the state in `/var/lib/reterminal-server/` in place.

## Uninstalling

```sh
sudo systemctl disable --now reterminal-server
sudo rm /etc/systemd/system/reterminal-server.service /usr/local/bin/reterminal-server
sudo systemctl daemon-reload
sudo rm -rf /var/lib/private/reterminal-server /var/lib/reterminal-server   # deletes saved content
```
