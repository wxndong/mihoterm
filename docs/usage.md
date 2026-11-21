# Keyboard and CLI Usage

## Managed Mode

Run `mihoterm` to start or reopen the managed proxy and dashboard. To select
a stored profile explicitly, run `mihoterm run PROFILE`. The proxy runs in
the background after you close the dashboard.

| Key | Action |
| --- | --- |
| `↑`, `↓` | Move within the focused list |
| `←`, `→`, `Tab` | Switch between groups and proxies |
| `Enter` | Open proxy confirmation. Press `y` to keep existing connections or `r` to reconnect only the selected group |
| `m` | Choose Rule, Global, or Direct mode and confirm |
| `d`, `p` | Probe the selected proxy or cycle probe targets |
| `c` | Open the read-only live connections view |
| `/` | Search the focused list |
| `r` | Request a background refresh |
| `s` | Open profile management |
| `q`, `Ctrl-C` | Close the dashboard |

```bash
mihoterm status
mihoterm doctor
mihoterm probe --proxy "Proxy A" --target Google
mihoterm exec -- curl https://example.com
mihoterm shell
mihoterm stop
```

Use `eval "$(mihoterm env)"` to set proxy variables for the current shell.
The optional Bash integration synchronizes those variables after managed
start and stop commands. See [managed runtime](managed-runtime.md) for
environment handling and [probes](probes.md) for what each target measures.

## Attach to Mihomo

To inspect or control an existing Mihomo instance, pass its controller URL
and secret file explicitly:

```bash
mihoterm --controller http://127.0.0.1:19090 \
  --secret-file ~/.config/mihoterm/controller.secret
```

If you set `MIHOTERM_CONTROLLER` and `MIHOTERM_SECRET_FILE` in your
environment, run `mihoterm attach`. The existing instance keeps running
after the dashboard closes.

See `mihoterm --help` for command options and [profiles](profiles.md) for
subscription configuration.
