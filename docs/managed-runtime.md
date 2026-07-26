# Managed Runtime

Managed mode is an explicit convenience for running one Mihomo child alongside
the TUI:

```console
$ mihoterm run primary
$ mihoterm run primary --mihomo /opt/mihomo/bin/mihomo
```

It is separate from attach mode. MihoTerm never adopts an existing process,
matches processes by name, modifies a service, or changes the stored profile.

## Startup sequence

1. Resolve and verify the selected Mihomo executable.
2. Create one mode `0700` per-run directory under the XDG runtime directory.
3. Reserve two distinct ephemeral TCP ports on `127.0.0.1`.
4. Read the owner-only managed profile and derive a hardened runtime YAML.
5. Generate a 256-bit controller secret from the operating system RNG.
6. Write the YAML and log as mode `0600` files.
7. Run Mihomo `-t` against the exact derived YAML.
8. Release the port reservations immediately before spawning Mihomo.
9. Wait for the authenticated loopback controller to report its version.
10. Open the TUI and show the mixed proxy endpoint in the header.

The validation and live process both receive an empty inherited environment,
a private home and temporary directory, and `umask 077`.

## Forced runtime overrides

The derived YAML preserves proxies, providers, policy groups, rules, and
ordinary outbound behavior. It overrides inbound and system-changing settings:

- `allow-lan` is false and `bind-address` is `127.0.0.1`;
- HTTP, SOCKS, redirect, and TProxy ports are disabled;
- one mixed proxy port and one controller port are dynamically allocated;
- TUN, iptables, NTP system writes, TUIC server, custom listeners, and tunnels
  are disabled;
- alternate TLS, Unix-socket, and named-pipe controllers are disabled;
- external UI downloads and the unauthenticated external DoH route are
  disabled;
- a new controller secret replaces any value in the stored profile.

The source profile is never rewritten.

## Shutdown behavior

`q`, `Ctrl-C`, SIGINT, SIGTERM, TUI errors, and ordinary unwinding stop and reap
the exact child before removing its per-run directory. The internal wrapper
also requests SIGTERM from the kernel if the MihoTerm parent disappears,
including after SIGKILL. A non-catchable parent failure can leave its private
per-run directory behind, but the Mihomo child still exits.

The runtime root itself is retained as an empty mode `0700` directory. MihoTerm
does not delete unknown directories or search for unrelated processes.

## Current limitations

- Managed mode is intentionally Linux-only.
- Relative local provider and rule files are resolved inside the isolated
  runtime home. Use HTTP providers or absolute owner-controlled paths until
  resource import is implemented.
- The dynamically allocated mixed proxy endpoint exists only while the TUI is
  running.
- Structural profile checks run before Mihomo, but Mihomo `-t` remains the
  complete schema authority.

The command-line behavior follows Mihomo's documented `-t`, `-d`, and `-f`
flags and its public
[`main.go`](https://github.com/MetaCubeX/mihomo/blob/Meta/main.go)
implementation.
