# Installation

## Portable archives

Portable release archives are the supported end-user distribution. They
contain:

- one statically linked `mihoterm` executable;
- one statically linked, unmodified official `mihomo` executable for the same
  CPU architecture;
- checksum-pinned standard Mihomo GeoIP and GeoSite data;
- a user-local installer and reversible Bash integration; and
- MihoTerm's MIT license, Rust dependency licenses, upstream GPL-3.0 licenses,
  and exact core and data provenance.

No Rust toolchain, dynamic application libraries, root access, systemd unit,
or distribution-specific runtime is required. A user-level systemd unit is an
optional integration installed by default when accepted at the prompt.

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
```

## User-local installation

Install for the current user:

```console
$ ./install.sh
$ exec "$SHELL" -l
$ mihoterm
```

The installer asks `Register MihoTerm to start automatically? [Y/n]` and
pressing Enter selects yes. Automated installation can choose deterministically:

```console
$ ./install.sh --autostart
$ ./install.sh --no-autostart
```

These options can be combined with `--no-shell`. Autostart uses
`~/.config/systemd/user/mihoterm.service` and an owner-controlled
`default.target.wants` link. On a server that must start Mihomo before the
first SSH or Codex session, an administrator must additionally run
`loginctl enable-linger USER`. The installer reports when lingering is off.

For a staged fleet upgrade, install the new immutable release and update the
command, shell, and service files without interrupting an active managed proxy:

```console
$ ./install.sh --autostart --defer-runtime-restart
```

The installation leaves the running managed process, its supervisor when
present, session identifier, credentials, and loopback ports unchanged. The
new runtime becomes active after an explicit `mihoterm stop` followed by
`mihoterm start`, a service restart, or the next service start. The option
does not invoke the previous release at all, so even an unhealthy or locked
older manager cannot block the install. It never adopts or preserves an
unrelated Mihomo process.

The installer creates an immutable version directory under
`$XDG_DATA_HOME/mihoterm/releases`, points
`$XDG_DATA_HOME/mihoterm/current` at that version, and creates
`~/.local/bin/mihoterm`. Existing non-MihoTerm files or links at those exact
paths are rejected instead of overwritten. Re-running the same installer is
idempotent; a same-version directory with different bytes is rejected.

When installing a different version, the complete new immutable directory is
prepared and verified first. If the old MihoTerm-managed proxy is running, the
installer records its profile, stops only its verified PID, atomically switches
`current`, and starts the new version with the same profile. If that start
fails, `current` is restored and the previous version is restarted. Unrelated
Mihomo and Codex processes are never searched or signalled.

Small marker-delimited blocks are added to `~/.profile`, an existing
`~/.bash_profile`, and `~/.bashrc`. The original files receive one
`.mihoterm.bak` backup. The Bash integration:

- makes `mihoterm` available from any working directory;
- synchronizes `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, and their lowercase
  forms after the proxy starts or stops;
- restores any proxy variables that existed before MihoTerm took control; and
- loads an already-running MihoTerm session in a newly opened Bash shell; and
- starts a missing session once when the user-enabled autostart link exists,
  while preserving an explicit `mihoterm stop`.

It never writes a port, username, password, or subscription URL into a shell
startup file. Those values are generated per session and read from an
owner-only runtime descriptor.

Use `--no-shell` to install the versioned files and command link without
editing shell startup files:

```console
$ ./install.sh --no-shell --no-autostart
$ export PATH="$HOME/.local/bin:$PATH"
$ eval "$(mihoterm env)"
```

## No-install mode

The extracted directory is also directly runnable:

```console
$ ./mihoterm
$ eval "$(./mihoterm env)"
```

Keep the directory intact. MihoTerm automatically selects the bundled
`mihomo` executable and copies the standard data into its private runtime
home.

## Which traffic uses MihoTerm

The default installation is rootless and per-user. Applications launched from
an integrated shell use MihoTerm when they honor the standard HTTP, HTTPS, or
SOCKS proxy environment variables. For explicit behavior, use:

```console
$ mihoterm exec -- COMMAND ARGUMENT...
$ mihoterm shell
```

MihoTerm does not silently enable TUN, change desktop-wide settings, or
intercept every process. Those actions require platform-specific privileges
and have a much larger failure surface, so they are not part of the default
installation.

## Uninstall

Remove the installed application, stop only its exact tracked Mihomo process,
and remove only installer-managed shell blocks:

```console
$ mihoterm uninstall
```

Profiles and configuration are preserved by default. To remove them too:

```console
$ mihoterm uninstall --purge
```

The `.mihoterm.bak` safety copies are intentionally retained. If the command
link or installation root is not owned by the MihoTerm installer, uninstall
stops with an error instead of deleting it.

## Source builds

Cloning the repository is a development workflow and requires Rust 1.88 or
newer. End users do not need the source tree or Cargo. Contributors should
follow [Development](development.md).

## Runtime data

Profiles and source descriptors are stored in owner-only XDG state
directories, normally `~/.local/state/mihoterm`. The session descriptor and
generated credentials use the XDG runtime directory when available. The
bundled Mihomo executable and MetaCubeX rule data are separate GPL-3.0 works;
their exact versions or commits, upstream checksums, sources, and licenses are
included in every archive.
