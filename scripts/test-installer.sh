#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d "$project_dir/target/installer-test.XXXXXXXX")"
trap 'rm -rf -- "$fixture"' EXIT

bundle="$fixture/bundle"
test_home="$fixture/home"
mock_bin="$fixture/mock-bin"
service_log="$fixture/systemctl.log"
export HOME="$test_home"
export XDG_DATA_HOME="$test_home/data"
export XDG_CONFIG_HOME="$test_home/config"
export XDG_STATE_HOME="$test_home/state"
export XDG_RUNTIME_DIR="$test_home/runtime"
export PATH="$mock_bin:$PATH"

# Hermetic: drop any MihoTerm session state inherited from the caller's shell so
# the installer lifecycle is exercised against a clean environment even when the
# user running this script has a live managed proxy active in their shell.
unset "${!MIHOTERM_@}"

install -d -m 700 \
  "$bundle/licenses" \
  "$bundle/shell" \
  "$bundle/systemd" \
  "$HOME" \
  "$mock_bin" \
  "$XDG_RUNTIME_DIR"
cat >"$mock_bin/systemctl" <<'EOF'
#!/bin/sh
printf '%s\n' "$*" >>"$MIHOTERM_TEST_SERVICE_LOG"
EOF
cat >"$mock_bin/loginctl" <<'EOF'
#!/bin/sh
printf '%s\n' yes
EOF
chmod 755 "$mock_bin/systemctl" "$mock_bin/loginctl"
export MIHOTERM_TEST_SERVICE_LOG="$service_log"
install -m 755 "$project_dir/target/release/mihoterm" "$bundle/mihoterm"
install -m 755 /bin/true "$bundle/mihomo"
install -m 755 "$project_dir/packaging/install.sh" "$bundle/install.sh"
install -m 644 "$project_dir/packaging/shell/mihoterm.sh" "$bundle/shell/mihoterm.sh"
install -m 644 "$project_dir/packaging/systemd/mihoterm.service" "$bundle/systemd/mihoterm.service"
for file in \
  geoip.metadb geoip.dat geosite.dat LICENSE THIRD_PARTY_NOTICES.md \
  THIRD-PARTY-LICENSES.html CORE-METADATA.txt DATA-METADATA.txt README.md; do
  : >"$bundle/$file"
done
: >"$bundle/licenses/Mihomo-GPL-3.0.txt"
: >"$bundle/licenses/meta-rules-dat-GPL-3.0.txt"
printf '%s\n' '# existing profile line' >"$HOME/.profile"
printf '%s\n' '# existing bashrc line' >"$HOME/.bashrc"

"$bundle/install.sh"
"$bundle/install.sh" --autostart

test -L "$HOME/.local/bin/mihoterm"
test -x "$XDG_DATA_HOME/mihoterm/current/mihoterm"
test -f "$XDG_CONFIG_HOME/systemd/user/mihoterm.service"
cmp -s "$bundle/systemd/mihoterm.service" "$XDG_CONFIG_HOME/systemd/user/mihoterm.service"
test "$(readlink "$XDG_CONFIG_HOME/systemd/user/default.target.wants/mihoterm.service")" = ../mihoterm.service
grep -Fq -- '--user daemon-reload' "$service_log"
test "$(grep -Fc '# >>> mihoterm managed >>>' "$HOME/.profile")" -eq 1
test "$(grep -Fc '# >>> mihoterm managed >>>' "$HOME/.bashrc")" -eq 1
grep -Fq '# existing profile line' "$HOME/.profile"
grep -Fq '# existing bashrc line' "$HOME/.bashrc"

HTTP_PROXY=http://127.0.0.1:65500 \
  bash --noprofile --norc -c '
    . "$HOME/.bashrc"
    type mihoterm >/dev/null
    test "$HTTP_PROXY" = http://127.0.0.1:65500
  '

install -d -m 700 "$XDG_CONFIG_HOME/mihoterm" "$XDG_STATE_HOME/mihoterm"
: >"$XDG_CONFIG_HOME/mihoterm/keep"
: >"$XDG_STATE_HOME/mihoterm/keep"
"$HOME/.local/bin/mihoterm" uninstall

test ! -e "$XDG_DATA_HOME/mihoterm"
test ! -e "$HOME/.local/bin/mihoterm"
test ! -e "$XDG_CONFIG_HOME/systemd/user/mihoterm.service"
test -e "$XDG_CONFIG_HOME/mihoterm/keep"
test -e "$XDG_STATE_HOME/mihoterm/keep"
test "$(grep -Fc '# >>> mihoterm managed >>>' "$HOME/.profile")" -eq 0
test "$(grep -Fc '# >>> mihoterm managed >>>' "$HOME/.bashrc")" -eq 0
grep -Fq '# existing profile line' "$HOME/.profile"
grep -Fq '# existing bashrc line' "$HOME/.bashrc"

"$bundle/install.sh" --no-autostart
"$HOME/.local/bin/mihoterm" uninstall --purge

test ! -e "$XDG_CONFIG_HOME/mihoterm"
test ! -e "$XDG_STATE_HOME/mihoterm"

make_fake_bundle() {
  local destination="$1"
  local version="$2"
  local fail_start="$3"
  cp -a "$bundle" "$destination"
  cat >"$destination/mihoterm" <<EOF
#!/bin/sh
case "\${1:-}" in
  --version) printf '%s\n' 'mihoterm $version' ;;
  status)
    if [ -e "\$XDG_RUNTIME_DIR/fake-running" ]; then
      printf '%s\n' 'MihoTerm proxy running | Mihomo fake | mode global | profile secondary | mixed 127.0.0.1:1 | pid 1'
    else
      exit 1
    fi
    ;;
  stop)
    printf '%s\n' '$version stop' >>"\$XDG_RUNTIME_DIR/fake-lifecycle.log"
    rm -f "\$XDG_RUNTIME_DIR/fake-running"
    ;;
  start)
    printf '%s\n' "$version start \${2:-}" >>"\$XDG_RUNTIME_DIR/fake-lifecycle.log"
    [ '$fail_start' = no ] || exit 1
    : >"\$XDG_RUNTIME_DIR/fake-running"
    ;;
  *) exit 0 ;;
esac
EOF
  chmod 755 "$destination/mihoterm"
}

upgrade_home="$fixture/upgrade-home"
export HOME="$upgrade_home"
export XDG_DATA_HOME="$upgrade_home/data"
export XDG_CONFIG_HOME="$upgrade_home/config"
export XDG_STATE_HOME="$upgrade_home/state"
export XDG_RUNTIME_DIR="$upgrade_home/runtime"
install -d -m 700 "$HOME" "$XDG_RUNTIME_DIR"

old_bundle="$fixture/old-bundle"
new_bundle="$fixture/new-bundle"
failed_bundle="$fixture/failed-bundle"
make_fake_bundle "$old_bundle" 0.0.1 no
make_fake_bundle "$new_bundle" 0.0.2 no
make_fake_bundle "$failed_bundle" 0.0.3 yes

"$old_bundle/install.sh" --no-shell --no-autostart
: >"$XDG_RUNTIME_DIR/fake-running"
"$new_bundle/install.sh" --no-shell --no-autostart
test "$(readlink "$XDG_DATA_HOME/mihoterm/current")" = "$XDG_DATA_HOME/mihoterm/releases/0.0.2"
sed -n '1p' "$XDG_RUNTIME_DIR/fake-lifecycle.log" | grep -Fxq '0.0.1 stop'
sed -n '2p' "$XDG_RUNTIME_DIR/fake-lifecycle.log" | grep -Fxq '0.0.2 start secondary'

if "$failed_bundle/install.sh" --no-shell --no-autostart; then
  echo "installer accepted an upgrade whose managed proxy failed to start" >&2
  exit 1
fi
test "$(readlink "$XDG_DATA_HOME/mihoterm/current")" = "$XDG_DATA_HOME/mihoterm/releases/0.0.2"
tail -n 3 "$XDG_RUNTIME_DIR/fake-lifecycle.log" | grep -Fxq '0.0.2 stop'
tail -n 2 "$XDG_RUNTIME_DIR/fake-lifecycle.log" | grep -Fxq '0.0.3 start secondary'
tail -n 1 "$XDG_RUNTIME_DIR/fake-lifecycle.log" | grep -Fxq '0.0.2 start secondary'

printf '%s\n' "installer lifecycle verified"
