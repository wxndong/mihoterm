# Development

## Requirements

- Linux
- Rust 1.88 or newer
- Git
- A live Mihomo instance only for opt-in integration tests

## Local validation

Run the same entry point before every pull request:

```console
$ ./scripts/ci-local.sh
```

The script checks formatting, linting, tests, the release build, and Git
whitespace. If `gitleaks` is installed, it also scans repository history.

Live integration tests must:

- bind only to loopback;
- request ephemeral ports from the operating system;
- use a dedicated temporary directory and controller secret;
- start and stop only their exact child process;
- remain opt-in and skip cleanly when Mihomo is unavailable.

The managed-runtime smoke profile must contain only fixture data. Verify the
mixed proxy through its displayed dynamic port, then confirm that the child,
both listeners, and the per-run directory disappear after `q`. Also confirm
that any pre-existing Mihomo PID is unchanged.

## Configuration locations

MihoTerm follows the XDG base directory specification:

- configuration: `$XDG_CONFIG_HOME/mihoterm`
- state: `$XDG_STATE_HOME/mihoterm`
- cache: `$XDG_CACHE_HOME/mihoterm`
- runtime data: `$XDG_RUNTIME_DIR/mihoterm`

Environment variables use the `MIHOTERM_` prefix.

`MIHOTERM_CONFIG` overrides the configuration file and
`MIHOTERM_STATE_DIR` overrides persistent state. Tests and development
commands should use dedicated temporary values instead of the user's normal
directories. `MIHOTERM_RUNTIME_DIR` overrides transient managed-runtime data,
and `MIHOTERM_MIHOMO` selects the Mihomo executable.
