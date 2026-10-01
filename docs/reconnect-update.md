# Reconnection and safe upgrades

## Stable endpoints and core supervision

The managed supervisor retains its saved loopback ports and credentials across
child recovery and explicit stop/start. An occupied saved port fails visibly;
it does not silently move clients to a new endpoint. Existing processes keep
the proxy environment they inherited, so new environment exports alone cannot
repair an older, different endpoint.

Local controller liveness is checked every five seconds with a one-second
request timeout. Two consecutive failures restart only the owned child. This
check is independent of external probe failures and the ten-minute network
recovery cooldown. Bounded restart backoff still applies. A short pause or a
concurrent profile reload does not immediately trigger a restart.

A child restart interrupts that child's established TCP streams. It preserves
the endpoint and credentials so applications can reconnect; applications must
still retry their failed requests. Profile reloads and ordinary node selection
preserve existing streams. Explicit scoped reconnection closes only matching
connections when requested by the user.

The supervisor reads the current stored profile identity when recovering a
child, including a profile selected after the supervisor originally started.
Service launchers should use `mihoterm supervise` without a fixed profile
argument when they should restore the user's latest saved selection.

## Subscription and health boundaries

Subscription downloads first try direct HTTPS, then the authenticated managed
proxy if available. Invalid content leaves the cached working profile intact.
An optional fallback policy is stored with the source and regenerated from
that selector's direct proxy members on every refresh. See [profiles](profiles.md).

TLS certificate and hostname verification remain enabled. Hosts with obsolete
trust stores must update their CA certificates or supply a trusted current
bundle with `SSL_CERT_FILE`; disabling verification is not a recovery method.
A new installation without a working managed proxy still needs direct access
or a local profile for its first import.

OpenAI API and ChatGPT Codex reachability are separate probes. Their expected
unauthenticated HTTP responses do not prove account access or a successful
model stream. The UI labels missing, expired and failed probe observations;
zero active connections is not evidence that a node is offline. See
[probes](probes.md) and [managed runtime](managed-runtime.md).

## Upgrade while tasks are active

Use the portable installer's `--defer-runtime-restart` option to stage the new
release while keeping the currently running proxy process and endpoint intact.
Staging does not activate new supervisor behavior. Activate it later during a
quiet period with an explicit managed service restart.

For a parallel migration, use separate state, runtime and configuration paths
for the candidate. Keep each old proxy serving its existing clients until
those clients finish or move to a new shell. Do not share a session descriptor
between two supervisors. A rollback must use locally available files and must
not depend on the proxy or a model request being reachable.

## Release acceptance

Run the local release gate and the real-core regressions against the executable
extracted from the final portable archive, in isolated runtime directories.

- Crash and repeated-hang recovery retain endpoint credentials and the latest
  hot-switched profile, including during the network recovery cooldown.
- A brief controller pause does not restart a healthy child.
- Node failover sends new requests through the surviving node while an existing
  stream remains open.
- Valid subscription refresh changes remote nodes without replacing listeners;
  malformed content, unavailable sources and untrusted TLS retain cached state.
- Occupied endpoints fail without identity drift; stop/start accepts the old
  client credentials.
- Installer staging leaves a running sentinel untouched.
- Published archive checksums match an independent download; existing production
  process identities and protected configuration hashes remain unchanged.

These gates validate bounded software recovery, not uninterrupted availability
of every external node or website. Release assets support Linux x86_64;
aarch64 and armv7 remain unvalidated.

## Scoped recovery

Enable an explicit policy on the active profile in Global mode:

```console
$ mihoterm profile policy primary --group 'My AI Group' --apply
$ mihoterm doctor
```

Activation preserves the current leaf when it is inside the boundary; it does
not rotate a working route to the generated fallback's first node.
The named selector defines the complete node boundary. Static nested groups are
expanded with cycle detection and deduplication; DIRECT/REJECT and unrelated
nodes are excluded. Dynamic providers or include-all membership are rejected
rather than silently widening or partially interpreting the boundary. The
subscription source, policy and active selection are persisted separately from
the generated runtime configuration. Refresh regenerates the group and keeps a
still-valid selection; missing or invalid policy members retain the previous
working revision. Explicit source replacement follows the compatibility rules
in [profiles](profiles.md).

The generated fallback checks the Codex target every 30 seconds. The managed
monitor checks every 15 seconds and requires two failed observations before
probing alternative leaves, four at a time. Each candidate must pass two checks
and the selected route is checked again before the selection is committed.
A bounded session-lock retry prevents controller checks from starving recovery.
Selection changes preserve listeners and established streams. There is a
120-second switch cooldown and a 300-second exclusion of the previous node.
When every eligible node fails, the monitor records a degraded result and never
escapes into another GLOBAL group. These timings are scheduling intervals, not
an end-to-end recovery deadline.

Mihomo v1.19.29 may return a positive delay even when the expected HTTP status
is not satisfied. MihoTerm verifies the per-URL `extra` health record as well
as the delay response; generic `alive` is insufficient. Missing target evidence
is unverified, not a successful probe.

## Optional Codex stream feedback

```console
$ mihoterm profile policy primary --group 'My AI Group' \
    --codex-log-db /absolute/path/to/.codex/logs_2.sqlite --apply
```

Enable this only for a local Codex diagnostic database whose writers use this
managed proxy. MihoTerm opens it read-only, consumes only new transport warnings
from `codex_core::responses_retry`, and retains no raw log bodies, credentials,
prompts or model output. Authentication, quota, rate-limit and explicit service
errors are excluded. Missing, busy or incompatible databases are reported as
`unavailable-probes-only`; network checks continue. This adapter targets an
internal diagnostic schema and is optional, not a requirement for proxy use.

Two transport/stream warnings within ten minutes can request a trial switch even
when short probes pass. The alternative must still pass repeated Codex probes.
At most two such trial switches are permitted within ten minutes; the normal
cooldown and node exclusion also apply. Trial switches remain explicitly
unverified for streaming: an unauthenticated HEAD result cannot establish a
successful authenticated model response. A node change helps subsequent
requests and does not migrate or close an existing TCP stream.

`doctor` reports the current scope, last observation, feedback availability and
recent recovery reasons. The bounded private `recovery.json` holds at most 32
sanitized events. Remove the local policy with:

```console
$ mihoterm profile policy primary --disable --apply
```

## Non-disruptive upgrade monitoring

For tasks that cannot tolerate a core restart, stage the new executable and use
`mihoterm watch` with the existing managed state/runtime paths. This command
reuses the current core and listeners and never adopts or terminates the old
supervisor. While active it reserves the older supervisor's network-recovery
cooldown, so only the scoped monitor changes routes; local child-liveness
supervision remains with the old owner. The reservation expires if the monitor
stops. Use a service manager to restart a failed watcher.

This is a transitional arrangement, not a claim that an old process has been
upgraded in memory. Once a core restart is acceptable, restart the primary
service on the new executable and stop the extra watcher. Never delete the
still-running release or the verified rollback copy before that cutover.

## Evidence and upstream context

- [Mihomo v1.19.29 URLTest implementation](https://github.com/MetaCubeX/mihomo/blob/v1.19.29/adapter/adapter.go)
  distinguishes transport success from per-URL expected-status success.
- [Mihomo nested fallback report](https://github.com/MetaCubeX/mihomo/issues/2588)
  motivates explicit leaf boundaries rather than interpreting a group's generic
  health as the health of all its descendants.
- [Codex stream regression discussion](https://github.com/openai/codex/issues/36059)
  includes reports of failures without corresponding proxy errors. These are
  community observations, not proof that every disconnect is a proxy fault.
- [Codex stale endpoint report](https://github.com/openai/codex/issues/47264)
  supports preserving inherited client endpoints across upgrades.
- [Codex workspace-routing regression report](https://github.com/openai/codex/issues/48476)
  distinguishes authenticated bootstrap requests from inference reachability.
  This is why release validation also runs a separate real Codex canary.
