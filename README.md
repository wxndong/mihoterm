# MihoTerm

A tiny, fast, keyboard-first TUI for [Mihomo](https://github.com/MetaCubeX/mihomo)
on Linux.

> **Status:** pre-alpha. The repository is being built in small, reviewed
> increments and is not ready for daily use yet.

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

## Planned capabilities

- Inspect version, mode, traffic, policy groups, proxies, providers, and
  connections.
- Select proxies and modes without editing YAML by hand.
- Probe Google, OpenAI, GitHub, or a custom HTTPS target.
- Import a subscription URL or local YAML as a named profile.
- Validate, atomically update, and roll back managed profiles.
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

Rust 1.85 or newer is required.

```console
$ ./scripts/ci-local.sh
```

See [Development](docs/development.md) and
[Architecture](docs/architecture.md) for the project contracts.

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
