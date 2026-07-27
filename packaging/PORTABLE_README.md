# MihoTerm Portable Bundle

This directory is self-contained. It does not require Rust, Cargo, systemd,
root access, or distribution-specific libraries.

Install MihoTerm for the current user:

```console
$ ./install.sh
$ exec "$SHELL" -l
$ mihoterm
```

On first run, paste an HTTPS subscription URL at the hidden prompt. MihoTerm
downloads and validates the profile, starts only the bundled Mihomo process,
and opens the TUI. Later runs reuse the protected local profile. No compiler,
runtime environment, root access, or system service is needed.

Run `./mihoterm` directly instead if no installation or shell integration is
desired.

Keep this directory intact: `mihoterm`, `mihomo`, and the three GeoIP/GeoSite
data files belong together. MihoTerm copies the data into a private per-run
home so a cold start does not depend on reaching GitHub before the proxy is
available. Press `q` or `Ctrl-C` to close the TUI; the authenticated loopback
proxy stays running. `mihoterm stop` stops only the exact process it started.

Advanced commands:

```console
$ ./mihoterm run PROFILE
$ ./mihoterm start
$ ./mihoterm stop
$ ./mihoterm exec -- COMMAND
$ ./mihoterm attach
$ ./mihoterm profile list
$ ./mihoterm --help
```

Press `s` in the managed TUI to inspect redacted subscription sources, add a
profile, replace a URL, or download a validated update.

Uninstall with `mihoterm uninstall`. Profiles and configuration are preserved
unless `mihoterm uninstall --purge` is explicitly requested.

User data follows the XDG base directory specification. It is normally stored
under `~/.local/state/mihoterm` and is not placed in this bundle directory.

The `mihoterm` executable is MIT-licensed and contains permissively licensed
Rust dependencies. The bundled, unmodified `mihomo` executable and
MetaCubeX rule data are GPL-3.0-licensed. See `THIRD_PARTY_NOTICES.md`,
`THIRD-PARTY-LICENSES.html`, both files under `licenses/`,
`CORE-METADATA.txt`, and `DATA-METADATA.txt`.
