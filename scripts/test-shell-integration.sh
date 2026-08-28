#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
fixture_root=$(mktemp -d "$project_dir/target/shell-test.XXXXXXXX")
trap 'rm -rf -- "$fixture_root"' EXIT

bundle=$fixture_root/bundle
config=$fixture_root/config
stub_log=$fixture_root/stub.log
stub_state=$fixture_root/running
mkdir -p "$bundle/shell" "$config/systemd/user/default.target.wants"
install -m 755 "$project_dir/tests/fixtures/mihoterm-shell-stub.sh" "$bundle/mihoterm"
install -m 644 "$project_dir/packaging/shell/mihoterm.sh" "$bundle/shell/mihoterm.sh"
ln -s ../mihoterm.service "$config/systemd/user/default.target.wants/mihoterm.service"

MIHOTERM_STUB_LOG=$stub_log \
MIHOTERM_STUB_STATE=$stub_state \
XDG_CONFIG_HOME=$config \
SHELL_FIXTURE=$bundle/shell/mihoterm.sh \
bash --noprofile --norc -c '
    source "$SHELL_FIXTURE"
    test "${MIHOTERM_PROXY_SESSION-}" = 11111111111111111111111111111111
    mihoterm stop
    test -z "${MIHOTERM_PROXY_SESSION-}"
'

test "$(awk '$0 == "start" { count += 1 } END { print count + 0 }' "$stub_log")" = 1

rm -f "$stub_log" "$config/systemd/user/default.target.wants/mihoterm.service"
MIHOTERM_STUB_LOG=$stub_log \
MIHOTERM_STUB_STATE=$stub_state \
XDG_CONFIG_HOME=$config \
SHELL_FIXTURE=$bundle/shell/mihoterm.sh \
bash --noprofile --norc -c '
    source "$SHELL_FIXTURE"
    test -z "${MIHOTERM_PROXY_SESSION-}"
'
test "$(awk '$0 == "start" { count += 1 } END { print count + 0 }' "$stub_log")" = 0

echo "shell integration tests passed"
