# Reconnection and safe upgrades

## Recovery behavior in alpha.6

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
