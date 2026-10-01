# Profile Management

MihoTerm stores validated Mihomo YAML as named profiles. Profile storage is
independent from attach mode: profile operations never reload, signal, or
change an attached Mihomo instance. Updating or replacing the active profile
in the managed TUI hot-reloads only MihoTerm's verified managed session.

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

An existing profile can be returned to its real subscription source while
keeping a local AI fallback policy:

```console
$ mihoterm profile source primary --url-file ~/.config/mihoterm/subscription.url \
    --fallback-group 'AI' --preferred-proxy 'Proxy A' --apply
```

This policy is saved beside the source URL. Every refresh rebuilds
`AI Auto` from the selector's current direct leaf nodes, preferring
the named node when it still exists. Nested groups and DIRECT/REJECT are not
expanded into the automatic fallback. The generated group is the selector's
first option and checks the Codex HTTPS target every 60 seconds, accepting its
unauthenticated 401/405 responses. A missing selector, no eligible nodes, or a
conflicting generated group name rejects a refresh and retains the cached
profile. When replacing a source in the CLI or TUI, an existing custom fallback
is retained only if it fits the new subscription. Otherwise MihoTerm removes
that inherited policy, keeps the new subscription's own groups, and reports
the change. It does not guess a replacement group or pool unrelated nodes.
An explicitly supplied new fallback must be valid; mistakes still reject the
replacement. Download or validation failures retain the old profile and source.
The replaced source and its policy remain available through rollback.

Profile IDs must match `[A-Za-z0-9][A-Za-z0-9_-]{0,39}`.

## Managed recovery policy

For continuous recovery, use `profile policy` on an existing source instead of
replacing its URL. `--apply` requires the active profile in Global mode and
persists the policy and selection. Activation keeps the currently selected leaf
when it belongs to the new boundary, otherwise it selects the generated fallback.
It does not change operating mode implicitly. Static nested members are supported by this policy;
legacy `--fallback-group` retains its direct-member behavior for compatibility.
See [scoped recovery](reconnect-update.md#scoped-recovery) for configuration,
feedback, failure handling and removal.

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
current profile active. Updating or replacing the active profile uses Mihomo's
non-forced reload path, preserves loopback listeners, and takes effect
immediately. It then probes the reloaded route and, when needed, tries only
remembered healthy choices or fallback/URL-test groups authored by the profile.
It never enumerates arbitrary leaf proxies. Any persistence or reload failure
restores the previous stored and runtime revision.

The equivalent CLI operations are:

```console
$ mihoterm profile list
$ mihoterm profile update primary
$ mihoterm profile update primary --apply
$ mihoterm profile rollback primary
$ mihoterm profile path primary
```

`update` reloads the stored source, validates the complete result, writes it
through private temporary files, and atomically replaces each stored file.
`--apply` additionally applies that revision to the managed session without
recreating listeners, runs the bounded post-reload recovery path, and rolls
back the stored revision if the runtime rejects it. The TUI performs this apply
automatically for the active profile.
Replacing a source follows the same validate-before-write contract. The
previous validated YAML and source descriptor become the rollback target.
`rollback` swaps both pairs, so the operation can be reversed once more.

Mutating commands acquire a non-blocking advisory lock. A concurrent command
fails visibly instead of racing another update.

## Validation and limits

- Subscription URLs must use HTTPS and cannot contain URL credentials or a
  fragment.
- Redirects are limited and may not downgrade from HTTPS.
- Downloads first try direct HTTPS. If that request fails and a verified managed
  session exists, CLI/TUI updates and automatic network recovery retry once
  through its authenticated loopback proxy. Environment proxy variables are
  not used. Initial setup without a managed session still requires direct access.
- HTTPS certificate and hostname verification remains enabled on both routes.
  A host with an outdated trust store must supply a valid current CA bundle;
  bypassing verification is not a supported recovery mechanism.
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
