# Optional Linux TUN mode

MihoTerm defaults to an authenticated loopback proxy. Processes that ignore proxy
variables can take another public route. TUN addresses that coverage gap; it does
not fix a bad upstream node, guarantee Cloudflare acceptance, or migrate an
existing model stream. See [Mihomo TUN documentation](https://wiki.metacubex.one/config/inbound/tun/).

The privilege split follows [ClashTUI](https://github.com/JohanChane/clashtui):
rootless proxy use works without TUN, while transparent routing requires an
administrator-provisioned core. MihoTerm never invokes sudo or takes capabilities
from a subscription. Have an administrator provision CAP_NET_ADMIN and CAP_NET_RAW on a trusted
Mihomo binary. If a separate protected binary is used, supply the same absolute
`--mihomo` path to `tun enable`. Its resolved path is persisted and reused by
future managed starts; an explicit `start --mihomo` overrides it.

```console
$ mihoterm tun status
$ mihoterm stop
$ mihoterm tun enable --mihomo /absolute/path/to/privileged/mihomo
$ mihoterm start --mihomo /absolute/path/to/privileged/mihomo
```

Enabling fails before saving settings if `/dev/net/tun` or core capabilities are
missing. An active managed session must be stopped before changing the setting.
Settings live in owner-only `tun.json`, independently of the subscription, and
are reused by managed starts and profile reloads. Subscription TUN settings are
still stripped. Only the current effective UID is included; private IPv4/IPv6,
loopback and link-local destinations are excluded to preserve LAN/SSH access.
The core uses mixed stack, automatic routing, interface detection and DNS
hijacking for captured port-53 traffic, with IPv6 enabled, without auto-redirect firewall rules.

```console
$ mihoterm stop
$ mihoterm tun disable
$ mihoterm start
```

These tests validate settings, sanitization and configuration generation. Packet
capture and route cleanup need verification on a host with the required core
capabilities. A rootless host must not report TUN as operational merely because
configuration validation passed.
