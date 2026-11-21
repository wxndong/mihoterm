<div align="center">
  <img src="docs/assets/mihoterm.png" alt="MihoTerm logo" width="112">
  <h1>MihoTerm</h1>
  <p><strong>A keyboard-first terminal UI for Mihomo on Linux.</strong></p>
  <p>
    <a href="https://github.com/wxndong/mihoterm/releases/latest"><img src="https://img.shields.io/github/v/release/wxndong/mihoterm?style=for-the-badge&amp;label=Release" alt="Latest release"></a>
    <a href="docs/installation.md"><img src="https://img.shields.io/badge/Linux-x86__64-38515e?style=for-the-badge&amp;logo=linux&amp;logoColor=white" alt="Linux x86-64"></a>
    <a href="https://github.com/wxndong/mihoterm/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/wxndong/mihoterm/ci.yml?branch=main&amp;style=for-the-badge&amp;label=CI" alt="CI status"></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/License-MIT-277e68?style=for-the-badge" alt="MIT license"></a>
  </p>
  <p>
    <a href="#quick-start">Quick Start</a> ·
    <a href="docs/README.md">Documentation</a> ·
    <a href="CHANGELOG.md">Changelog</a>
  </p>
</div>

MihoTerm lets you switch proxies and policy groups, inspect live connections,
and manage subscription profiles over SSH or in a local terminal. It can run
its own background [Mihomo](https://github.com/MetaCubeX/mihomo) instance or
attach to one you already maintain.

## Quick Start

Download the Linux x86-64 archive and `SHA256SUMS` from
[GitHub Releases](https://github.com/wxndong/mihoterm/releases/latest).
Verify the checksum, extract the archive, and run these commands from the
extracted directory:

```bash
./install.sh
exec "$SHELL" -l
mihoterm
```

MihoTerm asks for your subscription URL through a hidden prompt on first run.
The archive includes Mihomo and its geodata. Installation runs as the current
user. Press `q` to close the UI while the proxy keeps running.

Already running Mihomo? [Attach to its controller](docs/usage.md#attach-to-mihomo).
Using a local YAML file? [Import it as a profile](docs/profiles.md#sources).

## Everyday Use

| Action | Key or command |
| --- | --- |
| Choose a proxy and confirm the switch | `Enter` |
| Inspect connections and throughput | `c` |
| Manage profiles and update subscriptions | `s` |
| Run a command through the managed proxy | `mihoterm exec -- curl https://example.com` |
| Inspect the running session | `mihoterm status` or `mihoterm doctor` |

See [Keyboard and CLI usage](docs/usage.md) for the full controls, or
[recovery and upgrades](docs/reconnect-update.md) for long-running sessions.

## Development

`src/` holds the Rust application, `tests/` holds Rust and real-Mihomo tests,
`packaging/` holds portable installer assets, and `scripts/` contains build and
validation commands.

Run `./scripts/ci-local.sh` on Linux before submitting changes. See
[Development](docs/development.md) for toolchains and isolated test commands.

The default branch includes [unreleased DNS and restart fixes](CHANGELOG.md#unreleased).
The release badge tracks published portable archives.
