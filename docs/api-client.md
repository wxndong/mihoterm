# Mihomo API Client

The API client is the only module allowed to construct external-controller
requests. It supports version, runtime configuration, and proxy snapshots,
plus focused proxy selection, mode changes, managed configuration reloads,
and delay probes.

## Controller URL

The controller URL must:

- use `http` or `https`;
- contain no embedded username, password, query, or fragment;
- end at the root or path prefix where Mihomo API routes are exposed.

For example, both `http://127.0.0.1:9090` and
`https://controller.example/mihomo/` are valid.

## Authentication

When configured, the controller secret is sent as a bearer token. Its wrapper
redacts debug output, and raw response bodies are excluded from errors.

Prefer `MIHOTERM_SECRET` or a permission-restricted secret file over a
command-line argument.

## Failure behavior

Every request has a total timeout and a bounded JSON response. Errors expose an
operation, failure class, or HTTP status without copying the response body.
This keeps diagnostics useful without leaking controller data.

Dynamic policy-group and proxy names are encoded as individual URL path
segments. Mutating requests use typed JSON payloads and must pass through a
user confirmation in the application layer. Managed profile switches send a
complete safety-hardened configuration through Mihomo's forced reload endpoint;
stored subscription YAML is never sent directly.

Contract tests bind to an operating-system-assigned loopback port and never
connect to a live Mihomo instance.
