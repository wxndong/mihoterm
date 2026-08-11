# MihoTerm

A tiny, fast, keyboard-first TUI for [Mihomo](https://github.com/MetaCubeX/mihomo)
on Linux.

> **Status:** v0.1.0-alpha.3 is an early x86_64 Linux prerelease. Managed mode,
> user-local installation, and portable packaging have passed the project
> release gates.

MihoTerm is an independent client for the Mihomo external-controller API. It
does not provide proxy services, subscription content, or credentials.
Portable release archives include an unmodified, checksum-pinned official
Mihomo executable and its standard GeoIP/GeoSite data files so users do not
need to install a separate runtime or bootstrap data through an unconfigured
network.

## Goals

- Make common Mihomo operations fast with predictable keyboard controls.
- Stay small, responsive, and portable across mainstream Linux architectures.
- Keep controller credentials and profile sources out of logs and screenshots.
- Attach safely to an existing Mihomo instance or manage an explicitly started
  isolated instance.
- Present failures honestly instead of hiding them behind a single health score.

## Current capabilities

- Inspect the Mihomo version, mode, policy groups, proxies, health state, and
  latest recorded delay.
- Inspect live connections and real-time throughput in a read-only table.
- Select proxies and modes through an explicit confirmation step.
- Probe Google, OpenAI, or GitHub without changing the active proxy.
- Load additional HTTPS probe targets from a protected TOML configuration.
- Import Mihomo YAML from a protected subscription URL file or local file.
- Add, inspect in redacted form, replace, validate, update, and roll back named
  subscription profiles from the TUI or CLI.
- Keep one authenticated, user-owned Mihomo process running independently of
  the TUI, with private runtime files, random loopback ports, and exact-PID
  lifecycle control.
- Export standard HTTP, HTTPS, and SOCKS proxy variables, open a proxied shell,
  or run one proxied command without exposing generated credentials.
- Install into the user's home for invocation from any directory, add
  reversible Bash integration, and uninstall while preserving profiles by
  default.
- Guide first-run setup through a hidden terminal prompt and automatically find
  the Mihomo executable shipped beside MihoTerm.
- Build self-contained Linux release archives with no dynamic-library, Rust,
  Cargo, root, systemd, or first-start data-download requirement.

## Planned capabilities

- Inspect rule and proxy providers; close active connections.
- Validate portable archives on aarch64 and armv7 Linux.

## Non-goals

- Reimplementing proxy protocols, DNS, routing, or the Mihomo core.
- Providing, selling, recommending, or embedding subscription services.
- Transparently intercepting every process, modifying system-wide proxy
  settings, or requiring root privileges by default.
- Depending on systemd for the basic application lifecycle.

## Terminology

- **Subscription source:** a user-supplied URL or local YAML file.
- **Profile:** a validated local configuration derived from a subscription
  source.
- **Provider, policy group, and proxy:** terms used by the Mihomo API.

## Installation

End users should download a portable archive from
[GitHub Releases](https://github.com/wxndong/mihoterm/releases), not clone the
source repository. Choose the archive for the machine's CPU, verify it against
`SHA256SUMS`, extract it, and install it for the current user:

```console
$ ./install.sh
$ exec "$SHELL" -l
$ mihoterm
```

The archive is self-contained, so no compiler or runtime environment is
needed. On first run, MihoTerm asks for an HTTPS subscription URL using a
hidden terminal prompt, validates the downloaded profile, starts the bundled
Mihomo core on authenticated dynamic loopback ports, and opens the TUI. `q`
closes the TUI while the proxy remains available to the shell. No root access
or system service is used.

Running `./mihoterm` directly from the extracted directory remains supported
as a no-install portable mode.

See [Installation](docs/installation.md) for architecture selection,
user-local installation, shell behavior, uninstall, and the bundle's
third-party license boundary.

## Development

The source tree is for contributors. Rust 1.88 or newer is required.

```console
$ ./scripts/ci-local.sh
```

See [Development](docs/development.md) and
[Architecture](docs/architecture.md) for the project contracts. The
[API client contract](docs/api-client.md) defines controller URL,
authentication, and error-handling behavior.

## Current command line

MihoTerm starts guided managed mode by default:

```console
$ mihoterm
$ mihoterm run
$ mihoterm run primary
```

The background lifecycle and explicit integration commands are:

```console
$ mihoterm start
$ mihoterm status
$ mihoterm probe --proxy "Proxy A"
$ mihoterm probe --proxy "Proxy A" --target Google --target openai
$ eval "$(mihoterm env)"
$ mihoterm exec -- curl https://example.com
$ mihoterm shell
$ mihoterm stop
$ mihoterm uninstall
$ mihoterm uninstall --purge
```

`probe` runs every configured HTTPS target unless one or more `--target`
filters are supplied. Built-in names are matched case-insensitively, and
`openai` or `codex` selects `OpenAI / Codex`. The command uses the active
managed controller, or the explicit controller passed with `--controller`;
it never starts Mihomo or changes the selected proxy. Each target is reported
separately, and the command exits unsuccessfully if any requested probe fails.

The installer-managed Bash function automatically synchronizes the proxy
environment after `mihoterm`, `run`, `start`, and `stop`. It saves and restores
pre-existing proxy variables instead of permanently replacing them. Programs
that honor the standard proxy variables then use MihoTerm. Transparent
system-wide traffic capture is deliberately outside the default rootless mode.

Attaching to an existing Mihomo instance is always explicit:

```console
$ mihoterm --controller http://127.0.0.1:19090 \
    --secret-file ~/.config/mihoterm/controller.secret
$ mihoterm attach
$ mihoterm status
```

The TUI is keyboard-first:

- `Up` and `Down` move within the focused list.
- `Left`, `Right`, or `Tab` switch between policy groups and proxies.
- `Enter` selects a proxy after confirmation.
- `m` cycles to the next Mihomo mode after confirmation.
- `d` probes the selected proxy against the active target.
- `p` cycles through Google, OpenAI, and GitHub probe targets.
- `c` opens a read-only live connections view; the header shows real-time
  throughput.
- `/` searches the focused list.
- `r` requests a background refresh.
- `s` opens subscription profile management.
- In the profile page, `a` adds a source, `e` replaces its URL, and `u`
  downloads and validates an update.
- `q` or `Ctrl-C` closes the TUI without stopping the background proxy.

See [Probe semantics](docs/probes.md) before interpreting delay results and
[Live connections](docs/connections.md) for the connection view's polling and
throughput-rate semantics.

## Configuration and profiles

The default configuration file is
`$XDG_CONFIG_HOME/mihoterm/config.toml` (normally
`~/.config/mihoterm/config.toml`). It must be readable only by its owner.
Additional probe targets use this format:

```toml
[[probes]]
name = "Example"
url = "https://example.com/health"
expected = "204"
timeout_ms = 3000
```

Press `s` in managed TUI mode for the simplest workflow. URL input is hidden,
and the source list shows only a redacted origin such as
`https://example.com/…`; paths and query credentials are never rendered.
Changes to the active profile are stored safely and take effect after the
managed proxy is restarted.

The equivalent CLI accepts subscription URLs only through an owner-only file,
keeping them out of shell history and process listings:

```console
$ install -d -m 700 ~/.config/mihoterm
$ install -m 600 /dev/null ~/.config/mihoterm/subscription.url
$ $EDITOR ~/.config/mihoterm/subscription.url
$ mihoterm profile add primary \
    --url-file ~/.config/mihoterm/subscription.url
$ mihoterm profile update primary
$ mihoterm profile rollback primary
$ mihoterm profile list
```

A local Mihomo YAML file can be imported with
`mihoterm profile add primary --file ./profile.yaml`. Profile commands only
manage private local files; they never reload or modify an attached Mihomo
instance. See [Profile management](docs/profiles.md) for the complete storage
and failure contract.

To start or reuse the background proxy with a specific managed profile:

```console
$ mihoterm run primary
```

MihoTerm validates a derived configuration, disables system-changing and
additional inbound features, allocates fresh loopback controller and mixed
proxy ports, enables generated mixed-port authentication, and shows the port
in the TUI header. `q` leaves the proxy running; `mihoterm stop` stops only the
exact recorded process. Use `--mihomo /path/to/mihomo` when the executable is
not on `PATH`.

With no profile argument, managed mode selects `default`, selects the only
available profile, or opens first-run setup. It never edits the stored profile
and never adopts, reloads, or stops an existing Mihomo process. See
[Managed runtime](docs/managed-runtime.md) for its complete safety boundary
and current limitations.

## Security and privacy

MihoTerm has no telemetry. Secrets must never be committed, printed in logs, or
included in screenshots. See [SECURITY.md](SECURITY.md) for private
vulnerability reporting.

## Acknowledgements

The interaction and architecture are informed by other open-source projects.
See [ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md) for details and attribution.

## Disclaimer

MihoTerm is an independent project and is not affiliated with or endorsed by
MetaCubeX or Mihomo. Users are responsible for complying with applicable laws,
network policies, and service terms.

## License

MihoTerm is [MIT](LICENSE)-licensed. Portable archives also contain the
separate GPL-3.0-licensed Mihomo executable; see
[Third-party notices](THIRD_PARTY_NOTICES.md).
