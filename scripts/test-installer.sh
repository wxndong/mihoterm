#!/usr/bin/env bash
set -euo pipefail

project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d "$project_dir/target/installer-test.XXXXXXXX")"
trap 'rm -rf -- "$fixture"' EXIT

bundle="$fixture/bundle"
test_home="$fixture/home"
export HOME="$test_home"
export XDG_DATA_HOME="$test_home/data"
export XDG_CONFIG_HOME="$test_home/config"
export XDG_STATE_HOME="$test_home/state"
export XDG_RUNTIME_DIR="$test_home/runtime"

# Hermetic: drop any MihoTerm session state inherited from the caller's shell so
# the installer lifecycle is exercised against a clean environment even when the
# user running this script has a live managed proxy active in their shell.
unset "${!MIHOTERM_@}"

install -d -m 700 \
  "$bundle/licenses" \
  "$bundle/shell" \
  "$HOME" \
  "$XDG_RUNTIME_DIR"
install -m 755 "$project_dir/target/release/mihoterm" "$bundle/mihoterm"
install -m 755 /bin/true "$bundle/mihomo"
install -m 755 "$project_dir/packaging/install.sh" "$bundle/install.sh"
install -m 644 "$project_dir/packaging/shell/mihoterm.sh" "$bundle/shell/mihoterm.sh"
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
"$bundle/install.sh"

test -L "$HOME/.local/bin/mihoterm"
test -x "$XDG_DATA_HOME/mihoterm/current/mihoterm"
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
test -e "$XDG_CONFIG_HOME/mihoterm/keep"
test -e "$XDG_STATE_HOME/mihoterm/keep"
test "$(grep -Fc '# >>> mihoterm managed >>>' "$HOME/.profile")" -eq 0
test "$(grep -Fc '# >>> mihoterm managed >>>' "$HOME/.bashrc")" -eq 0
grep -Fq '# existing profile line' "$HOME/.profile"
grep -Fq '# existing bashrc line' "$HOME/.bashrc"

"$bundle/install.sh"
"$HOME/.local/bin/mihoterm" uninstall --purge

test ! -e "$XDG_CONFIG_HOME/mihoterm"
test ! -e "$XDG_STATE_HOME/mihoterm"

printf '%s\n' "installer lifecycle verified"
