# MihoTerm

A tiny, fast, keyboard-first TUI for [Mihomo](https://github.com/MetaCubeX/mihomo)
on Linux.

> **Status:** pre-alpha. Attach and safe-control modes are under active
> development and are not ready for daily use yet.

MihoTerm is an independent client for the Mihomo external-controller API. It
does not provide or bundle proxy services, subscription content, credentials,
or a Mihomo executable.

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
- Select proxies and modes through an explicit confirmation step.
- Probe Google, OpenAI, or GitHub without changing the active proxy.
- Load additional HTTPS probe targets from a protected TOML configuration.
- Import Mihomo YAML from a protected subscription URL file or local file.
- Validate, atomically update, and roll back named profiles without touching a
  running Mihomo instance.
- Start one explicitly requested Mihomo child with private runtime files,
  random loopback ports, a random controller secret, and automatic cleanup.

## Planned capabilities

- Inspect live traffic, providers, and connections.
- Ship static binaries for x86_64, aarch64, and armv7 Linux.

## Non-goals

- Reimplementing proxy protocols, DNS, routing, or the Mihomo core.
- Providing, selling, recommending, or embedding subscription services.
- Modifying system-wide proxy settings or requiring root privileges by
  default.
- Depending on systemd for the basic application lifecycle.

## Terminology

- **Subscription source:** a user-supplied URL or local YAML file.
- **Profile:** a validated local configuration derived from a subscription
  source.
- **Provider, policy group, and proxy:** terms used by the Mihomo API.

## Development

Rust 1.88 or newer is required.

```console
$ ./scripts/ci-local.sh
```

See [Development](docs/development.md) and
[Architecture](docs/architecture.md) for the project contracts. The
[API client contract](docs/api-client.md) defines controller URL,
authentication, and error-handling behavior.

## Current command line

MihoTerm attaches to `http://127.0.0.1:9090` by default:

```console
$ cargo run
```

Use an explicit controller or a permission-restricted secret file when needed:

```console
$ cargo run -- --controller http://127.0.0.1:19090 \
    --secret-file ~/.config/mihoterm/controller.secret
$ cargo run -- status
```

The TUI is keyboard-first:

- `Up` and `Down` move within the focused list.
- `Left`, `Right`, or `Tab` switch between policy groups and proxies.
- `Enter` selects a proxy after confirmation.
- `m` cycles to the next Mihomo mode after confirmation.
- `d` probes the selected proxy against the active target.
- `p` cycles through Google, OpenAI, and GitHub probe targets.
- `/` searches the focused list.
- `r` requests a background refresh.
- `q` or `Ctrl-C` exits.

See [Probe semantics](docs/probes.md) before interpreting delay results.

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

Subscription URLs are accepted only through an owner-only file, keeping them
out of shell history and process listings:

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

To start an isolated Mihomo child from a managed profile:

```console
$ mihoterm run primary
```

MihoTerm validates a derived configuration, disables system-changing and
additional inbound features, allocates fresh loopback controller and mixed
proxy ports, and shows the mixed port in the TUI header. `q`, `Ctrl-C`, and
normal termination stop only that exact child. Use `--mihomo /path/to/mihomo`
when the executable is not on `PATH`.

Managed mode never edits the stored profile and never adopts, reloads, or
stops an existing Mihomo process. See
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

[MIT](LICENSE)
