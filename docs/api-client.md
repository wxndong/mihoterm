# Mihomo API Client

The API client is the only module allowed to construct external-controller
requests. It currently supports read-only version, runtime configuration, and
proxy snapshots.

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

Contract tests bind to an operating-system-assigned loopback port and never
connect to a live Mihomo instance.
