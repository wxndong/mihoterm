# Architecture

## Product boundary

MihoTerm is a control plane for Mihomo. Mihomo remains responsible for proxy
protocols, DNS, routing, and packet forwarding.

```text
keyboard input ─┐
                ├─> application state ─> Mihomo API client
terminal view ──┘            │
                             ├─> profile store
                             └─> persistent session manager ─> Mihomo

installer ─> user-local release + reversible Bash environment integration
```

The terminal view renders immutable snapshots. Network, profile, and process
operations run outside the render path and report explicit results back to the
application state.

## Modes

### Attach mode

Attach mode connects to a user-selected external-controller endpoint. It never
starts, reloads, stops, or signals a Mihomo process unless the user explicitly
requests a supported API operation.

### Managed mode

Managed mode is the no-argument default. First-run onboarding creates a
protected profile, then starts an explicitly selected or bundle-adjacent
Mihomo executable with a dedicated runtime directory, dynamically allocated
loopback ports, generated controller and mixed-port credentials, and a durable
owner-only session descriptor. Mihomo runs independently of the TUI. MihoTerm
records and signals only the exact process it started after verifying both its
PID start time and command line.

## Safety invariants

1. No fixed development ports.
2. No global process matching such as `pkill` or `killall`.
3. No writes to an attached instance's configuration files.
4. Managed profile updates use validation, atomic replacement, and rollback.
5. Secrets never implement `Debug` in a form that reveals their value.
6. A panic or signal must restore the terminal before process exit.
7. Unsupported Mihomo API capabilities degrade visibly and safely.
8. Profile sources and probe URLs are never accepted as command-line values.
9. Concurrent profile mutations are rejected through an advisory file lock.
10. Managed mode never executes inherited `CLASH_*` overrides or lifecycle
    scripts.
11. Stored profiles are immutable inputs; managed mode runs a separate,
    hardened derivative.
12. The mixed proxy listener binds only to loopback and always requires
    generated per-session authentication.
13. Shell integration preserves prior proxy variables and removes only
    marker-delimited content that the installer owns.
14. Uninstall preserves profiles unless the user explicitly requests
    `--purge`.

## Module boundaries

- `app`: state transitions and commands.
- `tui`: terminal lifecycle, input mapping, and rendering.
- `mihomo`: typed API transport and compatibility handling.
- `profile`: protected source storage, bounded loading, validation, atomic
  update, and rollback.
- `runtime`: detached process lifecycle, authenticated session descriptors,
  proxy environments, and exact-PID stop behavior.
- `config`: XDG paths and user preferences.
- `onboarding`: deterministic profile selection and hidden first-run input.
- `tls`: one process-wide Ring provider for rustls clients.
