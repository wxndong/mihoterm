# MihoTerm Portable Bundle

This directory is self-contained. It does not require Rust, Cargo, systemd,
root access, or distribution-specific libraries.

Start MihoTerm:

```console
$ ./mihoterm
```

On first run, paste an HTTPS subscription URL at the hidden prompt. MihoTerm
downloads and validates the profile, starts only the bundled Mihomo process,
and opens the TUI. Later runs reuse the protected local profile.

Keep `mihoterm` and `mihomo` in the same directory. Press `q` or `Ctrl-C` to
exit. MihoTerm stops only the Mihomo child it started.

Advanced commands:

```console
$ ./mihoterm run PROFILE
$ ./mihoterm attach
$ ./mihoterm profile list
$ ./mihoterm --help
```

User data follows the XDG base directory specification. It is normally stored
under `~/.local/state/mihoterm` and is not placed in this bundle directory.

The `mihoterm` executable is MIT-licensed and contains permissively licensed
Rust dependencies. The bundled, unmodified `mihomo` executable is
GPL-3.0-licensed. See `THIRD_PARTY_NOTICES.md`,
`THIRD-PARTY-LICENSES.html`, `licenses/Mihomo-GPL-3.0.txt`, and
`CORE-METADATA.txt`.
