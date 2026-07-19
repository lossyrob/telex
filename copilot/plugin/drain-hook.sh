#!/bin/sh

neutral_decision='{}'
block_decision='{"decision":"block","reason":"Telex plugin/binary version skew: the telex binary resolved from PATH could not run `copilot drain`. Run `telex copilot drain --help` and `telex --json version`, then use `Get-Command telex` on Windows or `command -v telex` on POSIX to identify the PATH winner. Reinstall a matched plugin/binary release through the versioned installer, ensure its bin directory precedes stale shims such as cargo-installed copies, and restart Copilot. If intentionally rolling back the binary, roll back the plugin to the same release first. `TELEX_COPILOT_DRAIN=off` is only a temporary escape hatch."}'

if [ "${TELEX_COPILOT_DRAIN+x}" = x ]; then
    drain_setting=$(
        printf '%s' "$TELEX_COPILOT_DRAIN" \
            | tr '[:upper:]' '[:lower:]' \
            | sed 's/^[[:space:]]*//;s/[[:space:]]*$//'
    )
else
    drain_setting=
fi

case $drain_setting in
    off|0|false)
        printf '%s\n' "$neutral_decision"
        exit 0
        ;;
esac

if command telex --json copilot drain >/dev/null 2>&1; then
    printf '%s\n' "$neutral_decision"
else
    printf '%s\n' "$block_decision"
fi
exit 0
