# Development

## Requirements

- Linux
- Rust 1.88 or newer
- Git
- A live Mihomo instance only for opt-in integration tests

## Local validation

Run the same entry point before every pull request:

```console
$ ./scripts/ci-local.sh
```

Portable release builds additionally require Zig 0.16.0, cargo-zigbuild
0.23.0, cargo-about 0.9.1, cargo-audit 0.22.2, and the relevant Rust musl
target:

```console
$ cargo install cargo-zigbuild --version 0.23.0 --locked
$ cargo install cargo-about --version 0.9.1 --locked --features cli
$ cargo install cargo-audit --version 0.22.2 --locked
$ rustup target add x86_64-unknown-linux-musl
$ ./scripts/generate-licenses.sh
$ ./scripts/build-portable.sh x86_64-unknown-linux-musl
$ ./scripts/package-portable.sh x86_64-unknown-linux-musl
$ ./scripts/ci-release-local.sh x86_64-unknown-linux-musl
```

The packaging script downloads only the pinned official Mihomo core, standard
MetaCubeX GeoIP/GeoSite data, and their licenses. It verifies every SHA-256
value and writes release files under `dist/`. These tools are developer
requirements only; the resulting archive has no toolchain dependency.
`ci-release-local.sh` also rejects stale Rust dependency license notices and
non-reproducible archives.

Local validation defaults to one quarter of the CPUs reported as available by
`nproc` (at least one), so process affinity and cpuset limits are respected
where the platform exposes them. Cargo, Rayon, and test workers share that
budget. Set `MIHOTERM_BUILD_JOBS` explicitly only when a different limit is
appropriate.

The script checks formatting, linting, tests, documentation, the release build,
an isolated install/idempotency/uninstall lifecycle, Bash autostart/resync
behavior, and Git whitespace. If `gitleaks` is installed, it also scans
repository history.

Installer upgrade coverage must exercise `--defer-runtime-restart` with a live
sentinel process. The current release link and owned service file must advance,
while the complete status, process PID, mixed port, controller port, and
lifecycle log prove that no status, stop, or start command was issued by the
installer.

Live integration tests must:

- bind only to loopback;
- request ephemeral ports from the operating system;
- use a dedicated temporary directory and controller secret;
- start and stop only their exact child process;
- remain opt-in and skip cleanly when Mihomo is unavailable.

The managed-runtime smoke profile must contain only authorized test data.
Verify that unauthenticated mixed-port requests receive `407`, authenticated
HTTP and SOCKS requests succeed, and `q` leaves the exact tracked process
available. Then run `mihoterm stop` and confirm that the child, both listeners,
the descriptor, and the per-run directory disappear. Also confirm that any
pre-existing Mihomo PID is unchanged.

Supervisor fault injection must additionally terminate only the isolated child
PID, then verify that the supervisor PID, session identifier, mixed port, and
credentials remain stable while a new child PID becomes ready. Test profile
refresh through `profile update ID --apply` and confirm that its immediate
post-reload check recovers Codex reachability through a known-good or
profile-authored fallback without enumerating arbitrary leaf proxies. Run
`doctor` afterward as an independent health assertion.
Hold the session and desired-state locks from separate descriptors and verify
that foreground inspection returns a busy error without waiting, then releases
normally after each holder exits.

## Configuration locations

MihoTerm follows the XDG base directory specification:

- configuration: `$XDG_CONFIG_HOME/mihoterm`
- state: `$XDG_STATE_HOME/mihoterm`
- cache: `$XDG_CACHE_HOME/mihoterm`
- runtime data: `$XDG_RUNTIME_DIR/mihoterm`, the secure conventional
  `/run/user/UID/mihoterm`, or a durable state fallback when neither is usable

Environment variables use the `MIHOTERM_` prefix.

`MIHOTERM_CONFIG` overrides the configuration file and
`MIHOTERM_STATE_DIR` overrides persistent state. Tests and development
commands should use dedicated temporary values instead of the user's normal
directories. `MIHOTERM_RUNTIME_DIR` overrides transient managed-runtime data,
and `MIHOTERM_MIHOMO` selects the Mihomo executable.


## Real-core resilience gate

The opt-in regression uses only isolated loopback HTTP and DNS fixtures.
It tests an unresponsive UDP resolver with a working TCP alternative, explicit
group reconnect, no-op and rejected selections, TUI exit, live profile reload,
child crash recovery, and stop/start with the old client credentials.
It also verifies automatic node failover, the latest profile after child
recovery, brief controller stalls, and two repeated hangs during the external
network recovery cooldown.

    python3 scripts/test-resilience.py --binary "$CARGO_TARGET_DIR/release/mihoterm" \
      --mihomo /path/to/bundled/mihomo --root "$TMPDIR"

It stops only its recorded child/supervisor and retains private diagnostics
under the supplied root. Real subscription checks must use an explicitly
authorized external profile and must not copy its URL or credentials into Git.

Run the HTTPS subscription gate against the same extracted release binary:

    python3 scripts/test-subscription.py --binary /path/to/bundle/mihoterm \
      --mihomo /path/to/bundle/mihomo --root "$TMPDIR"

This gate needs Python 3 and OpenSSL. Its local CA, HTTPS server and authenticated
Mihomo instance are private fixtures. It verifies direct-download failure,
managed-proxy fallback, changed remote nodes, retained live streams, rejected
malformed updates, and rejected untrusted TLS certificates. It also checks
replacement by a subscription with different groups, one successful document
download, removal of an incompatible inherited policy, strict explicit-policy
validation, and restoration of the old source/policy after core rejection.

Run the scoped-recovery gate against the same extracted portable executable:

```console
$ python3 scripts/test-managed-recovery.py --binary /path/to/bundle/mihoterm \
    --mihomo /path/to/bundle/mihomo --root "$TMPDIR"
```

It uses a private test CA, local HTTPS/SSE servers, an explicit core wrapper
that trusts only the fixture CA, and an isolated diagnostic SQLite database.
It exercises wrong-status responses with positive delays, in-scope failover,
all-nodes-down behavior, nested membership, policy refresh/restart, stream
feedback, ignored service errors, cooldown, and policy removal. No production
credentials or remote model requests are involved in this gate. Add
`--legacy-binary /path/to/older/mihoterm` to validate the staged watcher upgrade
and confirm stopping the watcher leaves the older core and streams intact.
