# Compatibility

MihoTerm targets current Mihomo releases exposing the external-controller API.
Compatibility is capability-based: the client checks available responses and
disables unsupported actions rather than assuming every server has the newest
API.

Initial Linux release targets:

- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`
- `armv7-unknown-linux-musleabihf`

A separate executable is required for each CPU architecture. MihoTerm does not
bundle a Mihomo executable.
