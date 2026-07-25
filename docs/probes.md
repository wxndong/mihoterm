# Probe Semantics

MihoTerm probes measure target reachability and HTTP delay through one selected
Mihomo proxy. They are not bandwidth tests and do not prove that an
authenticated application workflow will succeed.

## Built-in targets

| Name | URL | Expected status |
| --- | --- | --- |
| Google | `https://www.gstatic.com/generate_204` | `204` |
| OpenAI / Codex | `https://api.openai.com/v1/models` | `401` |
| GitHub | `https://github.com/robots.txt` | `200` |

The OpenAI endpoint intentionally receives no API credential. An expected
`401` confirms that the OpenAI API route, TLS connection, and HTTP service are
reachable; it does not validate a Codex login, account entitlement, or model
request.

Mihomo performs the request through its
[`/proxies/:name/delay`](https://wiki.metacubex.one/en/api/#proxies) endpoint.
MihoTerm passes an explicit URL, timeout, and expected HTTP status.

## Interaction

- `p` selects the next target.
- `d` probes the highlighted proxy.
- Results remain separate per target; MihoTerm does not combine unrelated
  services into a single score.
- Probes are user-triggered and are not repeated automatically.

## Custom targets

The probe model validates a printable name, HTTPS URL, expected status or
range, and a timeout from 100 to 60000 milliseconds. Up to 32 additional
targets can be declared in the owner-only XDG configuration:

```toml
[[probes]]
name = "Example"
url = "https://example.com/health"
expected = "200-299"
timeout_ms = 3000
```

`expected` defaults to `200-299` and `timeout_ms` defaults to `5000`. Names
must be unique, including built-in names. An explicitly selected missing,
invalid, oversized, or group-readable configuration fails closed before the
TUI starts.

Probe URLs are treated as sensitive because a custom query may contain private
data. Their debug representation is redacted.
