# MihoTerm Bash integration. This file is sourced by the installer-managed
# block in ~/.bashrc.

_mihoterm_binary=$(
    CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." &&
        pwd -P
)/mihoterm

_mihoterm_save_proxy_environment() {
    [ -z "${MIHOTERM_PROXY_SESSION-}" ] || return 0
    [ -z "${MIHOTERM_PROXY_ENV_SAVED-}" ] || return 0

    export MIHOTERM_PROXY_ENV_SAVED=1
    for _mihoterm_name in \
        HTTP_PROXY HTTPS_PROXY http_proxy https_proxy \
        ALL_PROXY all_proxy NO_PROXY no_proxy; do
        if declare -p "$_mihoterm_name" >/dev/null 2>&1; then
            printf -v "MIHOTERM_SAVED_SET_${_mihoterm_name}" '%s' 1
            printf -v "MIHOTERM_SAVED_${_mihoterm_name}" '%s' "${!_mihoterm_name}"
            export "MIHOTERM_SAVED_SET_${_mihoterm_name}"
            export "MIHOTERM_SAVED_${_mihoterm_name}"
        fi
    done
    unset _mihoterm_name
}

_mihoterm_restore_proxy_environment() {
    [ -n "${MIHOTERM_PROXY_ENV_SAVED-}" ] || return 0

    for _mihoterm_name in \
        HTTP_PROXY HTTPS_PROXY http_proxy https_proxy \
        ALL_PROXY all_proxy NO_PROXY no_proxy; do
        _mihoterm_set_name="MIHOTERM_SAVED_SET_${_mihoterm_name}"
        _mihoterm_value_name="MIHOTERM_SAVED_${_mihoterm_name}"
        if [ "${!_mihoterm_set_name-}" = 1 ]; then
            printf -v "$_mihoterm_name" '%s' "${!_mihoterm_value_name}"
            export "$_mihoterm_name"
        else
            unset "$_mihoterm_name"
        fi
        unset "$_mihoterm_set_name" "$_mihoterm_value_name"
    done
    unset MIHOTERM_PROXY_ENV_SAVED _mihoterm_name
    unset _mihoterm_set_name _mihoterm_value_name
}

_mihoterm_clear_owned_proxy_environment() {
    if [ -n "${MIHOTERM_PROXY_SESSION-}" ]; then
        unset HTTP_PROXY HTTPS_PROXY http_proxy https_proxy
        unset ALL_PROXY all_proxy NO_PROXY no_proxy MIHOTERM_PROXY_SESSION
    fi
    _mihoterm_restore_proxy_environment
}

_mihoterm_sync_proxy_environment() {
    [ -x "$_mihoterm_binary" ] || return 0
    _mihoterm_environment=$("$_mihoterm_binary" env --if-running 2>/dev/null) ||
        return 0
    case "$_mihoterm_environment" in
        *"export MIHOTERM_PROXY_SESSION="*)
            _mihoterm_save_proxy_environment
            ;;
    esac
    eval "$_mihoterm_environment"
    unset _mihoterm_environment
    if [ -z "${MIHOTERM_PROXY_SESSION-}" ]; then
        _mihoterm_restore_proxy_environment
    fi
}

mihoterm() {
    local _mihoterm_first_argument _mihoterm_status
    _mihoterm_first_argument=${1-}
    case "$_mihoterm_first_argument" in
        "" | run | start)
            _mihoterm_save_proxy_environment
            ;;
    esac

    command "$_mihoterm_binary" "$@"
    _mihoterm_status=$?

    case "$_mihoterm_first_argument" in
        uninstall)
            _mihoterm_clear_owned_proxy_environment
            unset -f mihoterm
            unset -f _mihoterm_save_proxy_environment
            unset -f _mihoterm_restore_proxy_environment
            unset -f _mihoterm_clear_owned_proxy_environment
            unset -f _mihoterm_sync_proxy_environment
            unset _mihoterm_binary
            ;;
        env | exec | shell | attach | status | profile | --help | -h | --version | -V)
            ;;
        *)
            _mihoterm_sync_proxy_environment
            ;;
    esac
    return "$_mihoterm_status"
}

_mihoterm_sync_proxy_environment
