# Live connections

The connections view shows what the managed Mihomo core is doing right now: the
active TCP/UDP connections it is proxying, the proxy chain each one uses, and
how much each is transferring. It is a read-only control surface; it cannot
start, stop, or close connections.

## Opening the view

Press `c` on the dashboard. The connection table replaces the policy-group and
proxy panes. The header keeps its status line and adds a real-time aggregate
throughput reading.

| Key | Action |
| --- | --- |
| `c` | Open the connections view from the dashboard |
| `Up` / `Down` | Move the selection |
| `Home` / `End` | Jump to the first / last visible connection |
| `/` | Search the host, network, chain, and rule columns |
| `Esc` | Clear the search, or return to the dashboard when the search is empty |
| `Left` | Return to the dashboard |
| `r` | Request an immediate refresh |
| `q` / `Ctrl-C` | Close the TUI (the proxy keeps running) |

## Columns

- **Host** — the request host (SNI / `Host` header) when known, otherwise the
  destination `ip:port`.
- **Net** — `tcp` or `udp`.
- **Chain** — the proxy chain Mihomo used, from outer policy group to the
  outbound proxy.
- **Rule** — the routing rule that matched, with its payload (for example
  `DOMAIN-SUFFIX .com`).
- **Up** / **Down** — bytes transferred by that connection since it opened.

## Refresh and rate semantics

Connections and throughput are polled from the Mihomo
[`/connections`](https://wiki.metacubex.one/en/api/#connections) endpoint on the
same cadence as the rest of the TUI (the `--refresh-ms` option, default 1500 ms).
They are HTTP polls; MihoTerm does not open a WebSocket stream, so sub-second
updates are not available.

The header rate is derived from the delta of Mihomo's monotonic `uploadTotal`
and `downloadTotal` counters between two successful samples divided by the
elapsed time, not from a separate `/traffic` stream. If the connection endpoint
briefly fails the rate is hidden and the next successful sample re-baselines so
no delta is ever measured across a gap. Rate accuracy therefore depends on the
core exposing those counters, which all current Mihomo versions do.

The connection list is not paginated; every active connection is fetched each
cycle. A very large connection count is bounded only by the controller's 32 MiB
response limit.
