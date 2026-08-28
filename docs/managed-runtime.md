# Managed Runtime

Managed mode owns one persistent, per-user supervisor and its Mihomo child. It
is the default end-user mode, while attaching to an existing controller
remains explicit:

```console
$ mihoterm
$ mihoterm start
$ mihoterm status
$ mihoterm doctor
$ mihoterm doctor --repair
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

`mihoterm`, `run`, `start`, `shell`, and `exec` start the supervisor if it does
not already exist. A subsequent command reuses the same verified session. If
Mihomo exits unexpectedly, the supervisor restarts the exact child with bounded
backoff while preserving the session identifier, proxy listener, credentials,
and runtime configuration.
The managed TUI can switch to another stored profile through a confirmed,
transactional hot reload. It preserves the loopback ports and credentials,
updates the private session descriptor only after Mihomo accepts the new
configuration, and restores the previous configuration if persistence fails.
Requesting a different profile from a separate lifecycle command while one is
active still fails visibly.

The first cold managed start derives `mode: global` regardless of the mode
stored in the source profile. Later starts restore MihoTerm's durable desired
mode and valid selector choices. The source profile is never rewritten. A
confirmed mode change applies to the current session and persistent intent;
profile hot switches preserve the current runtime mode. Attach mode never
changes an external controller's mode.

Closing the TUI with `q` or `Ctrl-C` leaves the proxy running. This makes the
TUI a control surface rather than the lifetime owner. `mihoterm stop` and
`mihoterm uninstall` stop only the exact recorded process.

The owner-only session descriptor records:

- a random session identifier;
- the supervisor and child PIDs plus Linux `/proc` start-time values;
- the exact per-run configuration path;
- the active profile;
- loopback controller and mixed ports; and
- generated controller and mixed-proxy credentials.

Before status, environment export, or stop, MihoTerm verifies the descriptor,
PID start times, command lines, and runtime path. A recycled PID or altered
descriptor is rejected and never signaled. Version-1 descriptors from alpha.4
remain readable during upgrades.

## Startup sequence

1. Resolve and verify the selected Mihomo executable.
2. Resolve a stable owner-only runtime root from XDG, `/run/user/UID`, or the
   durable state fallback, then create one mode `0700` per-run directory.
3. Copy regular, size-bounded bundled GeoIP/GeoSite files into the private
   runtime home; symbolic links are rejected.
4. Reserve distinct ephemeral TCP ports on `127.0.0.1`.
5. Read the owner-only managed profile and derive a hardened runtime YAML.
6. Generate independent 256-bit controller and mixed-proxy credentials.
7. Write configuration, logs, and the session descriptor as mode `0600` files.
8. Run Mihomo `-t` against the exact derived YAML.
9. Release the port reservations immediately before spawning Mihomo.
10. Start Mihomo beneath the supervisor and wait for the authenticated loopback
    controller to report its version.
11. Atomically record the active profile revision, mode, selector choices, and
    session identities before reporting readiness.

Foreground commands use non-blocking session and desired-state locks. A
concurrent lifecycle operation reports busy state instead of hanging
indefinitely; the bounded startup path retries session contention until its
deadline and then terminates only the exact supervisor it launched.

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
user's service manager and defaults to yes. The `Type=simple` unit runs
`mihoterm supervise` in the foreground, restarts it only after failure, and
uses the service manager's own restart limiting. It does not adopt or search
for other Mihomo processes. `--no-autostart` keeps the on-demand model.

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
after lifecycle commands and restores earlier proxy variables after stop. On
shell startup it attempts one start only when the installer-managed autostart
link is enabled; an explicit `mihoterm stop` remains stopped. Credentials are
not printed by normal status commands or stored in `.bashrc`.

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

## Health and recovery

The supervisor performs low-frequency health checks only in Global mode. A
healthy result requires the OpenAI/Codex probe and at least one of the Google
or GitHub probes. Two failed observations are required before automatic
recovery. Recovery has a durable ten-minute cooldown and is limited to:

1. restarting an unresponsive exact child;
2. refreshing and validating the active subscription, then applying it with
   Mihomo `force=false` so listeners remain intact;
3. remembered healthy choices; and
4. fallback or URL-test groups already authored by the profile.

MihoTerm never scans arbitrary leaf nodes. Failed refresh/apply attempts roll
back the stored profile and runtime configuration. `mihoterm doctor` reports
the profile revision, controller, three probes, and same-UID processes that
inherited a different session marker. `doctor --repair` runs the same bounded
recovery immediately and never kills inherited client processes. An explicit
profile hot reload also performs the remembered/profile-authored selection
portion immediately, avoiding a multi-minute wait for the background interval.

## Current limitations

- Managed mode is Linux-only.
- Relative local provider and rule files are resolved inside the isolated
  runtime home. Use HTTP providers or absolute owner-controlled paths until
  resource import is implemented.
- Transparent TUN capture is intentionally not available in the rootless
  default mode.

The command-line behavior follows Mihomo's documented `-t`, `-d`, and `-f`
flags and its public
[`main.go`](https://github.com/MetaCubeX/mihomo/blob/Meta/main.go)
implementation.
