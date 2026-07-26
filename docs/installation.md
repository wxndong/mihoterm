# Installation

## Portable archives

Portable release archives are the supported end-user installation method.
They contain:

- one statically linked `mihoterm` executable;
- one statically linked, unmodified official `mihomo` executable for the same
  CPU architecture;
- checksum-pinned standard Mihomo GeoIP and GeoSite data for deterministic
  cold starts;
- MihoTerm's MIT license, Rust dependency licenses, upstream GPL-3.0 licenses,
  and exact core and data provenance.

No Rust toolchain, dynamic application libraries, root access, systemd unit,
or distribution-specific package is required.

Choose the archive that matches `uname -m`:

| `uname -m` | Archive suffix |
| --- | --- |
| `x86_64` or `amd64` | `linux-x86_64.tar.gz` |
| `aarch64` or `arm64` | `linux-aarch64.tar.gz` |
| `armv7l` | `linux-armv7.tar.gz` |

Verify and extract the downloaded archive:

```console
$ sha256sum -c SHA256SUMS
$ tar -xzf mihoterm-vVERSION-linux-ARCH.tar.gz
$ cd mihoterm-vVERSION-linux-ARCH
$ ./mihoterm
```

Keep the extracted directory intact. MihoTerm automatically selects the
`mihomo` executable and standard data beside itself, then copies the data into
the private per-run home. On first run, it accepts the subscription URL only
through a hidden interactive prompt; the URL does not enter shell history or
the process list.

## Optional user-local installation

The extracted directory can remain anywhere owned by the user. For a stable
command without copying either executable separately:

```console
$ install -d ~/.local/opt ~/.local/bin
$ mv mihoterm-vVERSION-linux-ARCH ~/.local/opt/mihoterm
$ ln -s ~/.local/opt/mihoterm/mihoterm ~/.local/bin/mihoterm
```

Ensure `~/.local/bin` is already on `PATH`, or run the executable by its full
path. No shell configuration is required when running it from the extracted
directory.

## Source builds

Cloning the repository is a development workflow and requires Rust 1.88 or
newer. End users do not need the source tree or Cargo. Contributors should
follow [Development](development.md).

## Runtime data

Profiles and source descriptors are stored in owner-only XDG state
directories, normally `~/.local/state/mihoterm`. Transient runtime data uses
the XDG runtime directory when available. MihoTerm does not install a service,
change system proxy settings, or require privileged ports.

The bundled Mihomo executable and MetaCubeX rule data are separate GPL-3.0
works. Their exact versions or commits, upstream checksums, sources, and
licenses are included in every archive.
