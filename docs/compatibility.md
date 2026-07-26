# Compatibility

MihoTerm targets current Mihomo releases exposing the external-controller API.
Compatibility is capability-based: the client checks available responses and
disables unsupported actions rather than assuming every server has the newest
API.

Initial Linux release targets:

- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`
- `armv7-unknown-linux-musleabihf`

A separate archive is required for each CPU architecture. Portable archives
pair the static MihoTerm build with the checksum-pinned official Mihomo build
for that architecture. The two executables remain separate programs and carry
their respective licenses.

The x86_64 archive is validated on CentOS 7 with Linux 3.10 as a conservative
compatibility floor. Both executables are static, and the canary requires no
dynamic application libraries. aarch64 and armv7 remain unsupported until
their exact archives pass equivalent hardware or emulated smoke tests.
