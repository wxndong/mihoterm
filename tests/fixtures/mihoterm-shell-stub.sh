#!/bin/sh
set -eu

printf '%s\n' "${1-}" >>"$MIHOTERM_STUB_LOG"
case "${1-}" in
    env)
        if [ -f "$MIHOTERM_STUB_STATE" ]; then
            printf '%s\n' \
                "export HTTP_PROXY='http://fixture'" \
                "export MIHOTERM_PROXY_SESSION='11111111111111111111111111111111'"
        else
            printf '%s\n' \
                'if [ -n "${MIHOTERM_PROXY_SESSION-}" ]; then' \
                'unset HTTP_PROXY MIHOTERM_PROXY_SESSION' \
                'fi'
        fi
        ;;
    start)
        : >"$MIHOTERM_STUB_STATE"
        ;;
    stop)
        rm -f "$MIHOTERM_STUB_STATE"
        ;;
esac
