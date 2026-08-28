# Architecture

## Product boundary

MihoTerm is a control plane for Mihomo. Mihomo remains responsible for proxy
protocols, DNS, routing, and packet forwarding.

```text
keyboard input ─┐
                ├─> application state ─> Mihomo API client
terminal view ──┘            │
                             ├─> profile + desired-state stores
                             └─> session supervisor ─> Mihomo
                                        │
                                        └─> bounded health recovery

installer ─> user-local release + reversible Bash/service integration
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
owner-only session descriptor. Mihomo runs as a child of a dedicated
supervisor independently of the TUI. The supervisor preserves the same
listener configuration across bounded child restarts. MihoTerm records and
signals only the exact processes it started after verifying PID start times
and command lines.

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
15. Optional autostart invokes the same exact-PID managed lifecycle and never
    establishes ownership from a process name.
16. Version replacement prepares the new immutable release before either
    stopping the old managed process or, when explicitly deferred, switching
    installed bytes without changing the active session. Immediate cutover
    rolls back if the new core cannot start.
17. Automated recovery requires two failed health observations, uses a durable
    cooldown, and considers only validated subscription refreshes, remembered
    known-good choices, and profile-authored fallback or URL-test groups.
    Explicit profile hot reloads immediately reuse the selection-only portion
    so accepted configuration does not wait for the periodic health interval.
18. Runtime path discovery is stable with or without `XDG_RUNTIME_DIR`; an
    existing durable fallback session remains authoritative.
19. Foreground lifecycle commands never block indefinitely behind session or
    desired-state locks; startup retries session contention only within its
    explicit deadline.

## Module boundaries

- `app`: state transitions and commands.
- `tui`: terminal lifecycle, input mapping, and rendering.
- `mihomo`: typed API transport and compatibility handling.
- `profile`: protected source storage, bounded loading, validation, atomic
  update, and rollback.
- `runtime`: supervised process lifecycle, authenticated session descriptors,
  bounded recovery, proxy environments, and exact-PID stop behavior.
- `state`: atomic owner-only desired profile, mode, selection, and recovery
  cooldown state.
- `doctor`: bounded same-UID `/proc` inspection for inherited session markers.
- `config`: XDG paths and user preferences.
- `onboarding`: deterministic profile selection and hidden first-run input.
- `tls`: one process-wide Ring provider for rustls clients.

## Resilience dependency trade-offs

MihoTerm reuses established components where they own the hard part:

- systemd supervises the foreground MihoTerm process when available, while the
  small Tokio supervisor keeps the no-systemd lifecycle functional;
- Mihomo remains the policy engine, including profile-authored fallback and
  URL-test groups; MihoTerm does not reimplement proxy ranking;
- `fs4` provides advisory locking, `reqwest` provides bounded HTTP, and the
  Ring dependency already used by rustls provides stable SHA-256 profile
  revision hashes.

Dedicated daemonization, retry, `/proc`, and multi-file atomic-write frameworks
were not added. Their generality would duplicate the existing exact-PID,
Tokio, and private-file primitives. The local implementations are deliberately
bounded: one `setsid`, a six-step restart schedule, same-UID `/proc` reads with
hard byte/process limits, and `fsync` plus atomic rename for each durable
descriptor.
