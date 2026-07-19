#!/bin/sh

allow_decision='{"decision":"allow"}'
block_decision='{"decision":"block","reason":"Telex plugin/binary version skew: the telex binary on PATH lacks or failed `copilot drain`, so this plugin cannot safely complete agentStop. Run `telex copilot drain --help` and `telex --json version`. Upgrade/reinstall through the versioned `telex upgrade --force` path or install a matched plugin/binary pair, then reload/restart Copilot. `TELEX_COPILOT_DRAIN=off` is only a temporary escape hatch."}'

case ${TELEX_COPILOT_DRAIN-} in
    [Oo][Ff][Ff]|0|[Ff][Aa][Ll][Ss][Ee])
        printf '%s\n' "$allow_decision"
        exit 0
        ;;
esac

if command telex --json copilot drain >/dev/null 2>&1; then
    printf '%s\n' "$allow_decision"
else
    printf '%s\n' "$block_decision"
fi
exit 0
