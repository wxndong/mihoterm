# Profile Management

MihoTerm stores validated Mihomo YAML as named profiles. Profile storage is
independent from attach mode: adding, updating, or rolling back a profile never
reloads, signals, or changes a running Mihomo instance. An explicit confirmed
switch in the managed TUI hot-reloads only MihoTerm's verified managed session.

## Sources

A profile has exactly one source:

- an HTTPS subscription URL loaded from an owner-only file; or
- a canonicalized local Mihomo YAML file.

URLs are deliberately not accepted as command-line arguments because shell
history and process listings are common disclosure paths. The URL file may
contain one URL followed by a newline and must not be accessible by group or
other users.

```console
$ install -d -m 700 ~/.config/mihoterm
$ install -m 600 /dev/null ~/.config/mihoterm/subscription.url
$ $EDITOR ~/.config/mihoterm/subscription.url
$ mihoterm profile add primary \
    --url-file ~/.config/mihoterm/subscription.url
```

For a local file:

```console
$ mihoterm profile add primary --file ./profile.yaml
```

Profile IDs must match `[A-Za-z0-9][A-Za-z0-9_-]{0,39}`.

## Operations

In managed TUI mode, press `s` to open the profile page:

- `Up` and `Down` select a profile.
- `Enter` switches the running managed session to the selected profile after confirmation.
- `a` adds a named HTTPS source.
- `e` replaces the selected profile's HTTPS source.
- `u` downloads and validates the selected stored source.
- `s` or `Esc` returns to the proxy dashboard.

Subscription input is hidden. The list renders only the HTTPS origin, such as
`https://example.com/…`; URL paths and query credentials are never drawn.
Adding or replacing a source downloads and validates the complete profile
before modifying stored files. Switching profiles hot-reloads the isolated
managed Mihomo process without changing its loopback ports or credentials,
then refreshes the dashboard immediately. A rejected profile leaves the
current profile active. Updating the active profile does not interrupt the
current connection and takes effect after switching away and back, or after
`mihoterm stop` followed by `mihoterm`.

The equivalent CLI operations are:

```console
$ mihoterm profile list
$ mihoterm profile update primary
$ mihoterm profile rollback primary
$ mihoterm profile path primary
```

`update` reloads the stored source, validates the complete result, writes it
through private temporary files, and atomically replaces each stored file.
Replacing a source follows the same validate-before-write contract. The
previous validated YAML becomes the rollback target. `rollback` swaps those
two YAML versions, so the operation can be reversed once more if needed.

Mutating commands acquire a non-blocking advisory lock. A concurrent command
fails visibly instead of racing another update.

## Validation and limits

- Subscription URLs must use HTTPS and cannot contain URL credentials or a
  fragment.
- Redirects are limited and may not downgrade from HTTPS.
- Environment proxy variables are not used by the profile downloader.
- The downloader identifies as `clash.meta` because many subscription
  services use that de facto identifier to return Mihomo-compatible YAML
  instead of an encoded generic URI list.
- URL files are limited to 16 KiB.
- Profile downloads and local YAML files are limited to 16 MiB.
- The response must be UTF-8 YAML with a mapping root.
- At least one of `proxies`, `proxy-providers`, or `proxy-groups` must exist.

This structural validation catches HTML error pages, encoded non-YAML
subscriptions, and unrelated YAML. Mihomo remains the authority for complete
schema and runtime validation.

## Storage

The default state directory is `$XDG_STATE_HOME/mihoterm` (normally
`~/.local/state/mihoterm`). Each profile directory is mode `0700`; its source
descriptor, current YAML, previous YAML, temporary files, and lock file are
mode `0600`.

Source descriptors contain the URL or canonical local path required for future
updates. MihoTerm does not print those values, include them in debug output, or
send telemetry. Profile YAML commonly contains credentials and should never be
committed.
