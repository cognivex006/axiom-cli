#!/bin/sh
# macOS x64 distribution wrapper.
#
# The entrypoint bootstrap consumes an Intel Mac release-set.json without a
# preinstalled CLI. The historical --cli/--from path delegates ecosystem install.

set -eu

[ "$(uname -s)" = "Darwin" ] && [ "$(uname -m)" = "x86_64" ] || {
    printf '%s\n' 'Install-AxiomCli.sh supports native macOS x64 only' >&2
    exit 9
}

usage() {
    cat <<'USAGE'
Usage: Install-AxiomCli.sh --cli FILE --from RELEASE_SET (--dry-run | --apply) [--approve-digest SHA256]
       Install-AxiomCli.sh --release-set DIR (--dry-run | --apply) [--approve-digest SHA256]

The --release-set path installs the two owned per-user entrypoints and manages
the user shell PATH. The --cli/--from path invokes the ecosystem installation
engine after the runtime bundle is available. Neither path writes system files.
USAGE
}

for argument in "$@"; do
    if [ "$argument" = --release-set ]; then
        SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
        exec sh "$SCRIPT_DIR/Manage-Entrypoints.sh" install "$@"
    fi
done

CLI=""
FROM=""
MODE=""
APPROVAL=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --cli|--from|--approve-digest)
            [ "$#" -ge 2 ] || { printf '%s\n' "missing value for $1" >&2; exit 2; }
            case "$1" in
                --cli) CLI=$2 ;;
                --from) FROM=$2 ;;
                --approve-digest) APPROVAL=$2 ;;
            esac
            shift 2 ;;
        --dry-run|--apply)
            [ -z "$MODE" ] || { printf '%s\n' 'choose exactly one mode' >&2; exit 2; }
            MODE=$1; shift ;;
        --help|-h) usage; exit 0 ;;
        *) printf '%s\n' "unknown option: $1" >&2; exit 2 ;;
    esac
done
[ -n "$CLI" ] && [ -x "$CLI" ] || { printf '%s\n' '--cli must name an executable axiom-cli' >&2; exit 3; }
case "$CLI" in
    /*) ;;
    */*) CLI="$(CDPATH= cd -- "$(dirname -- "$CLI")" && pwd)/$(basename -- "$CLI")" ;;
    *) CLI="$(pwd)/$CLI" ;;
esac
[ -n "$FROM" ] || { printf '%s\n' '--from is required' >&2; exit 2; }
[ -n "$MODE" ] || { printf '%s\n' 'choose --dry-run or --apply' >&2; exit 2; }
if [ "$MODE" = '--apply' ]; then
    [ -n "$APPROVAL" ] || { printf '%s\n' '--apply requires --approve-digest' >&2; exit 2; }
    exec "$CLI" install --from "$FROM" --apply --approve-digest "$APPROVAL" --json
fi
exec "$CLI" install --from "$FROM" --dry-run --json
