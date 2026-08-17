# Managed Runtime

Managed mode owns one persistent, per-user Mihomo process. It is the default
end-user mode, while attaching to an existing controller remains explicit:

```console
$ mihoterm
$ mihoterm start
$ mihoterm status
$ mihoterm stop
$ mihoterm run primary
$ mihoterm attach
```

With no profile argument, MihoTerm selects `default`, selects the only existing
profile, or starts guided first-run setup. With no `--mihomo` override, it uses
the executable beside MihoTerm before searching `PATH`.

Managed mode never adopts an existing process, matches processes by name,
modifies a system service, or changes another Mihomo instance.

## Lifecycle

`mihoterm`, `run`, `start`, `shell`, and `exec` start the managed process if it
does not already exist. A subsequent command reuses the same verified session.
The managed TUI can switch to another stored profile through a confirmed,
transactional hot reload. It preserves the loopback ports and credentials,
updates the private session descriptor only after Mihomo accepts the new
configuration, and restores the previous configuration if persistence fails.
Requesting a different profile from a separate lifecycle command while one is
active still fails visibly.

A cold managed start derives `mode: global` regardless of the mode stored in
the source profile. The source profile is never rewritten. A confirmed mode
change applies to the current session, and a profile hot switch preserves that
current runtime mode. Attach mode never changes an external controller's mode.

Closing the TUI with `q` or `Ctrl-C` leaves the proxy running. This makes the
TUI a control surface rather than the lifetime owner. `mihoterm stop` and
`mihoterm uninstall` stop only the exact recorded process.

The owner-only session descriptor records:

- a random session identifier;
- the PID and Linux `/proc` start-time value;
- the exact per-run configuration path;
- the active profile;
- loopback controller and mixed ports; and
- generated controller and mixed-proxy credentials.

Before status, environment export, or stop, MihoTerm verifies the descriptor,
PID start time, command line, and runtime path. A recycled PID or altered
descriptor is rejected and never signaled.

## Startup sequence

1. Resolve and verify the selected Mihomo executable.
2. Create one mode `0700` per-run directory under the XDG runtime directory.
3. Copy regular, size-bounded bundled GeoIP/GeoSite files into the private
   runtime home; symbolic links are rejected.
4. Reserve distinct ephemeral TCP ports on `127.0.0.1`.
5. Read the owner-only managed profile and derive a hardened runtime YAML.
6. Generate independent 256-bit controller and mixed-proxy credentials.
7. Write configuration, logs, and the session descriptor as mode `0600` files.
8. Run Mihomo `-t` against the exact derived YAML.
9. Release the port reservations immediately before spawning Mihomo.
10. Start Mihomo in a detached session and wait for the authenticated loopback
    controller to report its version.

The validation and live process receive a cleared environment, a private home
and temporary directory, and `umask 077`.

## Forced runtime overrides

The derived YAML preserves proxies, providers, policy groups, rules, and
ordinary outbound behavior. It overrides inbound and system-changing settings:

- `allow-lan` is false and `bind-address` is `127.0.0.1`;
- HTTP, SOCKS, redirect, and TProxy ports are disabled;
- one mixed proxy port and one controller port are dynamically allocated;
- the mixed port requires a generated username and password;
- TUN, iptables, NTP system writes, TUIC server, custom listeners, and tunnels
  are disabled;
- alternate TLS, Unix-socket, and named-pipe controllers are disabled;
- external UI downloads and the unauthenticated external DoH route are
  disabled; and
- generated controller credentials replace stored values.

When a profile enables DNS but omits a dedicated node-domain resolver, the
derived runtime reuses that profile's non-empty `default-nameserver` value as
`proxy-server-nameserver`. It also makes Mihomo's default
`respect-rules: false` behavior explicit when the profile does not set the
field. This prevents proxy-node DNS bootstrap from depending on the proxy it
is trying to start without selecting a resolver provider on the user's
behalf. Any explicit `respect-rules` or `proxy-server-nameserver` value is
preserved, including an explicitly empty node resolver list.

The source profile is never rewritten.

## Optional automatic startup

The portable installer offers to register `mihoterm.service` with the current
user's service manager and defaults to yes. The unit calls the same exact-PID
`start` and `stop` lifecycle described above; it does not adopt or search for
other Mihomo processes. `--no-autostart` keeps the original on-demand model.

On a shared server, boot-before-login requires lingering to be enabled for the
specific account by an administrator. Without lingering, the unit starts with
the user's first login. Applications that must not start before the proxy can
declare `After=mihoterm.service` and `Requires=mihoterm.service` in their own
user-unit configuration.

## Application integration

`mihoterm env` prints shell commands for the active authenticated endpoint.
The output includes uppercase and lowercase HTTP, HTTPS, and SOCKS variables,
`NO_PROXY` for local destinations, and a MihoTerm session marker.

```console
$ eval "$(mihoterm env)"
$ mihoterm exec -- curl https://example.com
$ mihoterm shell
```

The installer-managed Bash integration performs the `env` synchronization
after lifecycle commands and restores earlier proxy variables after stop.
Credentials are not printed by normal status commands or stored in `.bashrc`.

Environment variables affect applications that support them and are launched
from that environment. They are not transparent packet capture. MihoTerm does
not enable TUN or system-wide routing in the default mode.

## Stop behavior

Stop first sends SIGTERM to the verified PID and waits briefly. If the same
verified process remains, it sends SIGKILL and waits again. It then removes
only the recorded per-run directory and session descriptor. Unknown
directories and unrelated processes are never searched or deleted.

The runtime root itself remains as a mode `0700` directory. A stale descriptor
whose process no longer matches is cleaned without signaling the new PID
owner.

## Current limitations

- Managed mode is Linux-only.
- Relative local provider and rule files are resolved inside the isolated
  runtime home. Use HTTP providers or absolute owner-controlled paths until
  resource import is implemented.
- Updating the active subscription profile does not apply it immediately.
  Switch away and back in the managed TUI, or stop and restart after a
  validated update.
- Transparent TUN capture is intentionally not available in the rootless
  default mode.

The command-line behavior follows Mihomo's documented `-t`, `-d`, and `-f`
flags and its public
[`main.go`](https://github.com/MetaCubeX/mihomo/blob/Meta/main.go)
implementation.
