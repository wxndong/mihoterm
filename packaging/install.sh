#!/bin/sh
set -eu

umask 077

START_MARKER="# >>> mihoterm managed >>>"
END_MARKER="# <<< mihoterm managed <<<"

fail() {
    printf 'mihoterm installer: %s\n' "$*" >&2
    exit 1
}

note() {
    printf '%s\n' "$*"
}

shell_quote() {
    printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\"'\"'/g")"
}

require_absolute() {
    case "$1" in
        /*) ;;
        *) fail "$2 must be an absolute path" ;;
    esac
}

script_path=$0
case "$script_path" in
    /*) ;;
    *) script_path=$PWD/$script_path ;;
esac
script_dir=$(CDPATH= cd "$(dirname "$script_path")" && pwd -P)

: "${HOME:?HOME is required}"
require_absolute "$HOME" HOME

data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
state_home=${XDG_STATE_HOME:-"$HOME/.local/state"}
bin_dir=${MIHOTERM_BIN_DIR:-"$HOME/.local/bin"}
require_absolute "$data_home" XDG_DATA_HOME
require_absolute "$config_home" XDG_CONFIG_HOME
require_absolute "$state_home" XDG_STATE_HOME
require_absolute "$bin_dir" MIHOTERM_BIN_DIR

app_root=$data_home/mihoterm
releases_dir=$app_root/releases
current_link=$app_root/current
command_link=$bin_dir/mihoterm
profile_file=$HOME/.profile
bashrc_file=$HOME/.bashrc
bash_profile_file=$HOME/.bash_profile
systemd_user_dir=$config_home/systemd/user
autostart_unit=$systemd_user_dir/mihoterm.service
autostart_wants=$systemd_user_dir/default.target.wants
autostart_link=$autostart_wants/mihoterm.service

case "$app_root" in
    "$data_home"/mihoterm) ;;
    *) fail "refusing an unexpected installation path" ;;
esac

write_without_block() {
    input=$1
    output=$2
    awk -v start="$START_MARKER" -v end="$END_MARKER" '
        $0 == start {
            if (inside) {
                exit 3
            }
            inside = 1
            next
        }
        $0 == end {
            if (!inside) {
                print
            } else {
                inside = 0
            }
            next
        }
        !inside { print }
        END {
            if (inside) {
                exit 3
            }
        }
    ' "$input" >"$output"
}

replace_shell_file() {
    destination=$1
    contents=$2
    if [ -L "$destination" ]; then
        cp "$contents" "$destination"
        rm -f "$contents"
    elif [ -e "$destination" ]; then
        mode=$(stat -c '%a' "$destination") ||
            fail "cannot inspect $destination"
        chmod "$mode" "$contents"
        mv -f "$contents" "$destination"
    else
        chmod 600 "$contents"
        mv "$contents" "$destination"
    fi
}

backup_once() {
    file=$1
    if [ -e "$file" ] && [ ! -e "$file.mihoterm.bak" ]; then
        cp -p "$file" "$file.mihoterm.bak"
    fi
}

remove_shell_block() {
    file=$1
    [ -e "$file" ] || return 0
    temporary=$(mktemp "$file.mihoterm.XXXXXX") ||
        fail "cannot create a temporary shell file"
    if ! write_without_block "$file" "$temporary"; then
        rm -f "$temporary"
        fail "managed shell block in $file is malformed"
    fi
    replace_shell_file "$file" "$temporary"
}

append_profile_block() {
    file=$1
    backup_once "$file"
    temporary=$(mktemp "$file.mihoterm.XXXXXX") ||
        fail "cannot create a temporary shell file"
    if [ -e "$file" ]; then
        write_without_block "$file" "$temporary" || {
            rm -f "$temporary"
            fail "managed shell block in $file is malformed"
        }
    fi
    quoted_bin=$(shell_quote "$bin_dir")
    {
        printf '\n%s\n' "$START_MARKER"
        printf 'case ":$PATH:" in\n'
        printf '    *":%s:"*) ;;\n' "$bin_dir"
        printf '    *) PATH=%s:"$PATH" ;;\n' "$quoted_bin"
        printf 'esac\n'
        printf 'export PATH\n'
        printf '%s\n' "$END_MARKER"
    } >>"$temporary"
    replace_shell_file "$file" "$temporary"
}

append_bashrc_block() {
    file=$1
    hook=$2
    backup_once "$file"
    temporary=$(mktemp "$file.mihoterm.XXXXXX") ||
        fail "cannot create a temporary shell file"
    if [ -e "$file" ]; then
        write_without_block "$file" "$temporary" || {
            rm -f "$temporary"
            fail "managed shell block in $file is malformed"
        }
    fi
    quoted_hook=$(shell_quote "$hook")
    {
        printf '\n%s\n' "$START_MARKER"
        printf 'if [ -r %s ]; then\n' "$quoted_hook"
        printf '    . %s\n' "$quoted_hook"
        printf 'fi\n'
        printf '%s\n' "$END_MARKER"
    } >>"$temporary"
    replace_shell_file "$file" "$temporary"
}

owned_command_link() {
    [ -L "$command_link" ] || return 1
    [ "$(readlink "$command_link")" = "$current_link/mihoterm" ]
}

owned_autostart_unit() {
    [ -f "$autostart_unit" ] && [ ! -L "$autostart_unit" ] || return 1
    cmp -s "$script_dir/systemd/mihoterm.service" "$autostart_unit" && return 0
    if [ -f "$current_link/systemd/mihoterm.service" ] &&
        cmp -s "$current_link/systemd/mihoterm.service" "$autostart_unit"; then
        return 0
    fi
    for installed_unit in "$releases_dir"/*/systemd/mihoterm.service; do
        if [ -f "$installed_unit" ] && [ ! -L "$installed_unit" ] &&
            cmp -s "$installed_unit" "$autostart_unit"; then
            return 0
        fi
    done
    return 1
}

reload_user_manager() {
    command -v systemctl >/dev/null 2>&1 || return 0
    systemctl --user daemon-reload >/dev/null 2>&1 ||
        note "MihoTerm autostart was registered; the user service manager will load it at next login."
}

register_autostart() {
    [ "$bin_dir" = "$HOME/.local/bin" ] ||
        fail "autostart requires the default ~/.local/bin installation; use --no-autostart"
    if [ -e "$autostart_unit" ] || [ -L "$autostart_unit" ]; then
        owned_autostart_unit ||
            fail "refusing to replace an unowned service at $autostart_unit"
    fi
    install -d -m 700 "$systemd_user_dir"
    install -m 644 "$script_dir/systemd/mihoterm.service" "$autostart_unit"
    install -d -m 700 "$autostart_wants"
    if [ -e "$autostart_link" ] || [ -L "$autostart_link" ]; then
        [ -L "$autostart_link" ] &&
            [ "$(readlink "$autostart_link")" = ../mihoterm.service ] ||
            fail "refusing to replace an unowned autostart link at $autostart_link"
    else
        ln -s ../mihoterm.service "$autostart_link"
    fi
    reload_user_manager
    note "Registered MihoTerm to start with the user service manager."
    if command -v loginctl >/dev/null 2>&1; then
        linger=$(loginctl show-user "$(id -un)" --property=Linger --value 2>/dev/null || true)
        if [ "$linger" = no ]; then
            note "Boot-before-login requires an administrator to run: loginctl enable-linger $(id -un)"
        fi
    fi
}

remove_autostart() {
    if [ -e "$autostart_unit" ] || [ -L "$autostart_unit" ]; then
        owned_autostart_unit ||
            fail "refusing to remove an unowned service at $autostart_unit"
    fi
    if [ -e "$autostart_link" ] || [ -L "$autostart_link" ]; then
        [ -L "$autostart_link" ] &&
            [ "$(readlink "$autostart_link")" = ../mihoterm.service ] ||
            fail "refusing to remove an unowned autostart link at $autostart_link"
        rm -f "$autostart_link"
    fi
    if [ -e "$autostart_unit" ]; then
        rm -f "$autostart_unit"
    fi
    reload_user_manager
}

remove_installation() {
    purge=${1:-no}

    if [ -e "$autostart_unit" ] || [ -L "$autostart_unit" ]; then
        owned_autostart_unit ||
            fail "refusing to remove an unowned service at $autostart_unit"
    fi

    if [ -x "$current_link/mihoterm" ]; then
        "$current_link/mihoterm" stop >/dev/null 2>&1 ||
            fail "could not stop the managed proxy; installation was preserved"
    fi

    remove_autostart

    remove_shell_block "$profile_file"
    remove_shell_block "$bashrc_file"
    if [ -e "$bash_profile_file" ]; then
        remove_shell_block "$bash_profile_file"
    fi

    if [ -e "$command_link" ] || [ -L "$command_link" ]; then
        if owned_command_link; then
            rm -f "$command_link"
        else
            fail "refusing to remove an unowned command at $command_link"
        fi
    fi

    if [ -e "$app_root" ] || [ -L "$app_root" ]; then
        [ ! -L "$app_root" ] ||
            fail "refusing to remove a symbolic-link installation root"
        rm -rf "$app_root"
    fi

    if [ "$purge" = yes ]; then
        config_dir=$config_home/mihoterm
        state_dir=$state_home/mihoterm
        case "$config_dir:$state_dir" in
            "$config_home"/mihoterm:"$state_home"/mihoterm) ;;
            *) fail "refusing unexpected data paths" ;;
        esac
        rm -rf "$config_dir" "$state_dir"
        if [ -n "${XDG_RUNTIME_DIR:-}" ]; then
            require_absolute "$XDG_RUNTIME_DIR" XDG_RUNTIME_DIR
            runtime_dir=$XDG_RUNTIME_DIR/mihoterm
            case "$runtime_dir" in
                "$XDG_RUNTIME_DIR"/mihoterm) rm -rf "$runtime_dir" ;;
                *) fail "refusing an unexpected runtime path" ;;
            esac
        fi
        note "Removed MihoTerm, shell integration, configuration, and profiles."
    else
        note "Removed MihoTerm and its shell integration."
        note "Profiles and configuration were preserved. Use --purge to remove them."
    fi
}

install_bundle() {
    shell_integration=yes
    autostart=ask
    defer_runtime_restart=no
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --no-shell) shell_integration=no ;;
            --autostart) autostart=yes ;;
            --no-autostart) autostart=no ;;
            --defer-runtime-restart) defer_runtime_restart=yes ;;
            *) fail "usage: ./install.sh [--no-shell] [--autostart|--no-autostart] [--defer-runtime-restart]" ;;
        esac
        shift
    done
    if [ "$autostart" = ask ]; then
        if [ -t 0 ]; then
            printf 'Register MihoTerm to start automatically? [Y/n] '
            IFS= read -r answer || answer=
            case "$answer" in
                "" | y | Y | yes | YES | Yes) autostart=yes ;;
                n | N | no | NO | No) autostart=no ;;
                *) fail "please answer yes or no" ;;
            esac
        else
            autostart=yes
        fi
    fi

    for required in \
        mihoterm mihomo geoip.metadb geoip.dat geosite.dat \
        LICENSE THIRD_PARTY_NOTICES.md THIRD-PARTY-LICENSES.html \
        CORE-METADATA.txt DATA-METADATA.txt README.md install.sh \
        shell/mihoterm.sh systemd/mihoterm.service licenses/Mihomo-GPL-3.0.txt \
        licenses/meta-rules-dat-GPL-3.0.txt; do
        [ -f "$script_dir/$required" ] ||
            fail "portable bundle is missing $required"
    done
    [ -x "$script_dir/mihoterm" ] || fail "mihoterm is not executable"
    [ -x "$script_dir/mihomo" ] || fail "mihomo is not executable"
    if [ -e "$command_link" ] || [ -L "$command_link" ]; then
        owned_command_link ||
            fail "refusing to replace an unowned command at $command_link"
    fi
    if [ -e "$autostart_unit" ] || [ -L "$autostart_unit" ]; then
        owned_autostart_unit ||
            fail "refusing to replace an unowned service at $autostart_unit"
    fi
    if [ "$autostart" = yes ] && [ "$bin_dir" != "$HOME/.local/bin" ]; then
        fail "autostart requires the default ~/.local/bin installation; use --no-autostart"
    fi

    version=$("$script_dir/mihoterm" --version | awk 'NR == 1 { print $2 }')
    case "$version" in
        "" | *[!A-Za-z0-9._+-]*) fail "cannot determine a safe release version" ;;
    esac

    release_dir=$releases_dir/$version
    install -d -m 700 "$releases_dir" "$bin_dir"
    if [ ! -d "$release_dir" ]; then
        staging=$releases_dir/.install-$$
        [ ! -e "$staging" ] ||
            fail "temporary installation directory already exists"
        install -d -m 755 "$staging/licenses" "$staging/shell" "$staging/systemd"
        install -m 755 "$script_dir/mihoterm" "$staging/mihoterm"
        install -m 755 "$script_dir/mihomo" "$staging/mihomo"
        install -m 755 "$script_dir/install.sh" "$staging/install.sh"
        install -m 644 "$script_dir/shell/mihoterm.sh" "$staging/shell/mihoterm.sh"
        install -m 644 "$script_dir/systemd/mihoterm.service" "$staging/systemd/mihoterm.service"
        for file in \
            geoip.metadb geoip.dat geosite.dat LICENSE THIRD_PARTY_NOTICES.md \
            THIRD-PARTY-LICENSES.html CORE-METADATA.txt DATA-METADATA.txt README.md; do
            install -m 644 "$script_dir/$file" "$staging/$file"
        done
        install -m 644 \
            "$script_dir/licenses/Mihomo-GPL-3.0.txt" \
            "$staging/licenses/Mihomo-GPL-3.0.txt"
        install -m 644 \
            "$script_dir/licenses/meta-rules-dat-GPL-3.0.txt" \
            "$staging/licenses/meta-rules-dat-GPL-3.0.txt"
        mv "$staging" "$release_dir"
    else
        [ -x "$release_dir/mihoterm" ] ||
            fail "installed MihoTerm executable is not usable"
        [ -x "$release_dir/mihomo" ] ||
            fail "installed Mihomo executable is not usable"
        for file in \
            mihoterm mihomo geoip.metadb geoip.dat geosite.dat \
            LICENSE THIRD_PARTY_NOTICES.md THIRD-PARTY-LICENSES.html \
            CORE-METADATA.txt DATA-METADATA.txt README.md install.sh \
            shell/mihoterm.sh systemd/mihoterm.service licenses/Mihomo-GPL-3.0.txt \
            licenses/meta-rules-dat-GPL-3.0.txt; do
            cmp -s "$script_dir/$file" "$release_dir/$file" ||
                fail "installed release $version differs from this bundle"
        done
    fi

    previous_release=
    previous_binary=
    previous_profile=
    was_running=no
    runtime_stopped=no
    deferred_upgrade=no
    if [ -e "$current_link" ] && [ ! -L "$current_link" ]; then
        fail "refusing to replace non-link path $current_link"
    fi
    if [ -L "$current_link" ]; then
        previous_release=$(readlink "$current_link")
        case "$previous_release" in
            "$releases_dir"/*) ;;
            *) fail "refusing an unexpected current release link" ;;
        esac
        previous_binary=$previous_release/mihoterm
        [ -x "$previous_binary" ] || fail "installed current release is not usable"
        if [ "$previous_release" != "$release_dir" ]; then
            if [ "$defer_runtime_restart" = yes ]; then
                deferred_upgrade=yes
            else
                previous_status=$($previous_binary status 2>/dev/null || true)
                if [ -n "$previous_status" ]; then
                    was_running=yes
                    previous_profile=$(printf '%s\n' "$previous_status" |
                        sed -n 's/.* | profile \([A-Za-z0-9_-][A-Za-z0-9_-]*\) | mixed .*/\1/p')
                    [ -n "$previous_profile" ] ||
                        fail "cannot determine the active profile before upgrade"
                    "$previous_binary" stop >/dev/null ||
                        fail "could not stop the previous managed proxy; installation was preserved"
                    runtime_stopped=yes
                fi
            fi
        fi
    fi
    current_temporary=$app_root/.current-$$
    ln -s "$release_dir" "$current_temporary"
    mv -Tf "$current_temporary" "$current_link"

    if [ -e "$command_link" ] || [ -L "$command_link" ]; then
        owned_command_link ||
            fail "refusing to replace an unowned command at $command_link"
        rm -f "$command_link"
    fi
    ln -s "$current_link/mihoterm" "$command_link"

    if [ "$runtime_stopped" = yes ]; then
        if ! "$current_link/mihoterm" start "$previous_profile" >/dev/null; then
            rollback_temporary=$app_root/.rollback-$$
            ln -s "$previous_release" "$rollback_temporary"
            mv -Tf "$rollback_temporary" "$current_link"
            if "$current_link/mihoterm" start "$previous_profile" >/dev/null; then
                fail "the new managed proxy failed to start; restored the previous release"
            fi
            fail "the new managed proxy failed to start and the previous release could not be restored"
        fi
        note "Replaced the running MihoTerm proxy with version $version."
    elif [ "$deferred_upgrade" = yes ]; then
        note "Installed MihoTerm $version with the runtime restart deferred."
        note "The installer did not inspect, stop, or start the previous runtime; cut over with an explicit stop/start or service restart."
    fi

    if [ "$autostart" = yes ]; then
        register_autostart
    else
        remove_autostart
        note "MihoTerm autostart is disabled."
    fi

    if [ "$shell_integration" = yes ]; then
        append_profile_block "$profile_file"
        if [ -e "$bash_profile_file" ]; then
            append_profile_block "$bash_profile_file"
        fi
        append_bashrc_block "$bashrc_file" "$current_link/shell/mihoterm.sh"
    fi

    note "Installed MihoTerm $version in $release_dir"
    note "Open a new Bash shell, then run: mihoterm"
    if [ "$shell_integration" = no ]; then
        note "Add $bin_dir to PATH manually."
    fi
}

case "${1:-install}" in
    install)
        if [ "$#" -gt 0 ]; then
            shift
        fi
        install_bundle "$@"
        ;;
    uninstall)
        shift
        purge=no
        if [ "${1:-}" = "--purge" ]; then
            purge=yes
            shift
        fi
        [ "$#" -eq 0 ] || fail "usage: install.sh uninstall [--purge]"
        remove_installation "$purge"
        ;;
    --no-shell | --autostart | --no-autostart | --defer-runtime-restart)
        install_bundle "$@"
        ;;
    *)
        fail "usage: ./install.sh [--no-shell] [--autostart|--no-autostart] [--defer-runtime-restart] | ./install.sh uninstall [--purge]"
        ;;
esac
