#!/bin/sh
# Uninstall-AxiomCli.sh - axiom-cli Linux x64 per-user uninstaller.
#
# Removes only artifacts this distribution owns: the entrypoint it placed, its
# generations, its ownership record and the systemd user unit it generated.
# Per-user state is preserved unless --purge is given with an approved plan
# digest, because the distribution contract and docs/30-DISTRIBUTION-AND-INSTALLERS
# both require an explicit, separate approval before private state is deleted.
#
# Usage: Uninstall-AxiomCli.sh [--plan | --apply --approve-digest HEX] [options]

set -u

AC_EXIT_SUCCESS=0
AC_EXIT_VALIDATION=2
AC_EXIT_NOT_FOUND=3
AC_EXIT_AUTHORIZATION=5
AC_EXIT_IO=8

AC_SELF_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
# shellcheck source=AxiomCli.Linux.Common.sh
. "$AC_SELF_DIR/AxiomCli.Linux.Common.sh"

ac_usage() {
    cat <<'USAGE'
axiom-cli Linux x64 per-user uninstaller

USAGE:
    Uninstall-AxiomCli.sh [OPTIONS]

OPTIONS:
    --install-root DIR     install root (default $XDG_DATA_HOME/axiom-cli)
    --bin-dir DIR          directory holding the entrypoint (default $HOME/.local/bin)
    --state-dir DIR        per-user state directory (default $XDG_STATE_HOME/axiom-cli)
    --purge                also delete per-user state and the install root
    --plan                 compute the removal plan and stop (default)
    --apply                perform the removal (requires --approve-digest)
    --approve-digest HEX   approval digest bound to the computed plan digest
    --json                 emit the machine-readable install-result envelope
    --envelope-out FILE    also write the envelope to FILE
    --quiet                suppress progress diagnostics on stderr
    --help                 show this help and exit 0

EXIT CODES (owned by axiom-specs/docs/16-CLI-AND-CONTROL-API.md section 6):
    0 success, 2 validation, 3 not found, 5 authorization, 8 I/O

NOTES:
    User data is preserved by default. Only this distribution's own files are
    removed; a file this installer did not write is never deleted.
USAGE
}

AC_INSTALL_ROOT=""
AC_BIN_DIR=""
AC_STATE_DIR=""
AC_PURGE=0
AC_APPLY=0
AC_QUIET=0
AC_APPROVE_DIGEST=""
AC_ENVELOPE_OUT=""
AC_ARG_ERROR=""

while [ $# -gt 0 ]; do
    case "$1" in
        --install-root)
            [ $# -ge 2 ] || { AC_ARG_ERROR="--install-root requires a value"; ac_validate_fail; }
            AC_INSTALL_ROOT="$2"; shift 2 ;;
        --bin-dir)
            [ $# -ge 2 ] || { AC_ARG_ERROR="--bin-dir requires a value"; ac_validate_fail; }
            AC_BIN_DIR="$2"; shift 2 ;;
        --state-dir)
            [ $# -ge 2 ] || { AC_ARG_ERROR="--state-dir requires a value"; ac_validate_fail; }
            AC_STATE_DIR="$2"; shift 2 ;;
        --approve-digest)
            [ $# -ge 2 ] || { AC_ARG_ERROR="--approve-digest requires a value"; ac_validate_fail; }
            AC_APPROVE_DIGEST="$2"; shift 2 ;;
        --envelope-out)
            [ $# -ge 2 ] || { AC_ARG_ERROR="--envelope-out requires a value"; ac_validate_fail; }
            AC_ENVELOPE_OUT="$2"; shift 2 ;;
        --purge) AC_PURGE=1; shift ;;
        --plan) shift ;;
        --apply) AC_APPLY=1; shift ;;
        --json) shift ;;
        --quiet) AC_QUIET=1; shift ;;
        --help|-h) ac_usage; exit "$AC_EXIT_SUCCESS" ;;
        *) AC_ARG_ERROR="unknown option: $1"; ac_validate_fail ;;
    esac
done

if [ -n "$AC_APPROVE_DIGEST" ]; then
    case "$AC_APPROVE_DIGEST" in
        *[!0-9a-fA-F]*) AC_ARG_ERROR="--approve-digest must be hexadecimal"; ac_validate_fail ;;
    esac
    if [ "${#AC_APPROVE_DIGEST}" -ne 64 ]; then
        AC_ARG_ERROR="--approve-digest must be 64 hexadecimal characters"; ac_validate_fail
    fi
fi

AC_DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
AC_STATE_HOME="${XDG_STATE_HOME:-$HOME/.local/state}"
[ -n "$AC_INSTALL_ROOT" ] || AC_INSTALL_ROOT="$AC_DATA_HOME/axiom-cli"
[ -n "$AC_BIN_DIR" ] || AC_BIN_DIR="$HOME/.local/bin"
[ -n "$AC_STATE_DIR" ] || AC_STATE_DIR="$AC_STATE_HOME/axiom-cli"
case "$AC_INSTALL_ROOT" in /*) ;; *) AC_INSTALL_ROOT="$(pwd)/$AC_INSTALL_ROOT" ;; esac
case "$AC_BIN_DIR" in /*) ;; *) AC_BIN_DIR="$(pwd)/$AC_BIN_DIR" ;; esac
case "$AC_STATE_DIR" in /*) ;; *) AC_STATE_DIR="$(pwd)/$AC_STATE_DIR" ;; esac

AC_TMP="$(mktemp -d 2>/dev/null || printf '%s/axiom-cli-uninstall-%s' "${TMPDIR:-/tmp}" "$$")"
[ -d "$AC_TMP" ] || mkdir -p -- "$AC_TMP"
AC_LIST_ARTIFACTS="$AC_TMP/list-artifacts"; : > "$AC_LIST_ARTIFACTS"
AC_LIST_UNVERIFIED="$AC_TMP/list-unverified"; : > "$AC_LIST_UNVERIFIED"
AC_LIST_COMPONENTS="$AC_TMP/list-components"; : > "$AC_LIST_COMPONENTS"
AC_LIST_REFUSALS="$AC_TMP/list-refusals"; : > "$AC_LIST_REFUSALS"
AC_LIST_PRESERVED="$AC_TMP/list-preserved"; : > "$AC_LIST_PRESERVED"
AC_LIST_REMOVED="$AC_TMP/list-removed"; : > "$AC_LIST_REMOVED"
AC_LIST_LIMITATIONS="$AC_TMP/list-limitations"; : > "$AC_LIST_LIMITATIONS"

ac_list_add() { printf '%s\n' "$2" >> "$1"; }
ac_list_json() {
    ac_json_acc="["
    ac_json_first=1
    while IFS= read -r ac_json_line; do
        [ -n "$ac_json_line" ] || continue
        if [ "$ac_json_first" = "0" ]; then ac_json_acc="$ac_json_acc,"; fi
        ac_json_acc="$ac_json_acc$ac_json_line"
        ac_json_first=0
    done < "$1"
    printf '%s]' "$ac_json_acc"
}
ac_refusal() {
    ac_list_add "$AC_LIST_REFUSALS" "$(printf '{"check":%s,"artifact":%s,"expected_sha256":%s,"actual_sha256":%s,"reason":%s}' \
        "$(ac_json_string "$1")" "$(ac_json_string "$2")" \
        "$(ac_json_string_or_null "$3")" "$(ac_json_string_or_null "$4")" "$(ac_json_string "$5")")"
}
ac_removed() {
    ac_list_add "$AC_LIST_REMOVED" "$(printf '{"path":%s,"kind":%s}' "$(ac_json_string "$1")" "$(ac_json_string "$2")")"
}
ac_preserved() {
    ac_list_add "$AC_LIST_PRESERVED" "$(printf '{"path":%s,"reason":%s}' "$(ac_json_string "$1")" "$(ac_json_string "$2")")"
}
ac_limitation() { ac_list_add "$AC_LIST_LIMITATIONS" "$(ac_json_string "$1")"; }

AC_EV_OPERATION="uninstall"
AC_EV_OUTCOME="planned"
AC_EV_EXIT="$AC_EXIT_SUCCESS"
AC_EV_STATUS="ok"
AC_EV_MESSAGE=""
AC_EV_RETRYABLE=0
AC_EV_DRY_RUN=1
AC_EV_MUTATED=0
AC_EV_PLAN_DIGEST=""
AC_EV_APPROVED_DIGEST=""
AC_EV_TXN=""
AC_EV_RELEASE_SET_JSON="null"
AC_EV_PATH_RULE_JSON="$(printf '{"scope":"none","file":null,"entry":"%s","applied":false,"machine_wide_change":false,"previous_present":false}' "$(ac_json_escape "$AC_BIN_DIR")")"
AC_EV_SERVICE_JSON='{"kind":"none","unit_name":null,"unit_path":null,"owner":null,"registered":false,"removed":false,"daemon_reloaded":false,"reason":"no service registration is recorded by this envelope"}'

ac_emit_result() {
    ac_env_reset
    ac_put schema_version "$AC_ENVELOPE_SCHEMA_VERSION"
    ac_put spec_version "$(ac_json_string "$AC_SPEC_VERSION")"
    ac_put envelope_kind '"install-result"'
    ac_put operation "$(ac_json_string "$AC_EV_OPERATION")"
    ac_put outcome "$(ac_json_string "$AC_EV_OUTCOME")"
    ac_put exit_code "$AC_EV_EXIT"
    ac_put status "$(ac_json_string "$AC_EV_STATUS")"
    ac_put message "$(ac_json_string "$AC_EV_MESSAGE")"
    ac_put retryable "$(ac_json_bool "$AC_EV_RETRYABLE")"
    ac_put platform '"linux-x64"'
    ac_put host "$(ac_host_json)"
    ac_put install_root "$(ac_json_string_or_null "$AC_INSTALL_ROOT")"
    ac_put bin_dir "$(ac_json_string_or_null "$AC_BIN_DIR")"
    ac_put dry_run "$(ac_json_bool "$AC_EV_DRY_RUN")"
    ac_put mutated "$(ac_json_bool "$AC_EV_MUTATED")"
    ac_put plan_digest "$(ac_json_string_or_null "$AC_EV_PLAN_DIGEST")"
    ac_put approved_digest "$(ac_json_string_or_null "$AC_EV_APPROVED_DIGEST")"
    ac_put transaction_id "$(ac_json_string_or_null "$AC_EV_TXN")"
    ac_put interrupted_install_recovered 'false'
    ac_put release_set "$AC_EV_RELEASE_SET_JSON"
    ac_put artifacts "$(ac_list_json "$AC_LIST_ARTIFACTS")"
    ac_put unverified_artifacts "$(ac_list_json "$AC_LIST_UNVERIFIED")"
    ac_put components "$(ac_list_json "$AC_LIST_COMPONENTS")"
    ac_put refusals "$(ac_list_json "$AC_LIST_REFUSALS")"
    ac_put path_rule "$AC_EV_PATH_RULE_JSON"
    ac_put service_registration "$AC_EV_SERVICE_JSON"
    ac_put preserved "$(ac_list_json "$AC_LIST_PRESERVED")"
    ac_put removed "$(ac_list_json "$AC_LIST_REMOVED")"
    ac_put elevation_required 'false'
    ac_put shell "$(ac_json_string "$AC_SHELL_NAME")"
    ac_put limitations "$(ac_list_json "$AC_LIST_LIMITATIONS")"
    ac_put generated_at "$(ac_json_string "$(ac_now_utc)")"
    ac_put request_id "$(ac_json_string "$(ac_request_id)")"
    ac_emit_envelope
}

ac_finish() {
    ac_emit_result
    rm -rf -- "$AC_TMP" 2>/dev/null || true
    exit "$AC_EV_EXIT"
}

ac_validate_fail() {
    AC_EV_OUTCOME="refused"; AC_EV_EXIT="$AC_EXIT_VALIDATION"; AC_EV_STATUS="refused"
    AC_EV_MESSAGE="${AC_ARG_ERROR:-invalid arguments}"
    ac_refusal "arguments" "-" "" "" "$AC_EV_MESSAGE"
    ac_finish
}

ac_refuse() {
    AC_EV_OUTCOME="refused"; AC_EV_EXIT="$1"; AC_EV_STATUS="refused"; AC_EV_MESSAGE="$2"
    ac_refusal "$3" "-" "" "" "$2"
    ac_finish
}

# ---------------------------------------------------------------------------
# Discover what this distribution owns
# ---------------------------------------------------------------------------

AC_OWNER_MARKER_PATH="$AC_INSTALL_ROOT/$AC_OWNER_MARKER"
AC_RECORDED_VERSION=""
AC_RECORDED_PLAN_DIGEST=""
AC_OWNED=0
if [ -f "$AC_OWNER_MARKER_PATH" ]; then
    AC_OWNED=1
    AC_RECORDED_VERSION="$(sed -n 's/^version=//p' "$AC_OWNER_MARKER_PATH" | head -n 1)"
    AC_RECORDED_PLAN_DIGEST="$(sed -n 's/^plan_digest=//p' "$AC_OWNER_MARKER_PATH" | head -n 1)"
fi
AC_ENTRYPOINT_PRESENT=0
if [ -f "$AC_BIN_DIR/axiom-cli" ]; then
    AC_ENTRYPOINT_PRESENT=1
fi

# Only unit files carrying this installer's generated header are touched.
AC_AC_UNIT_FILES=""
AC_UNIT_DIR="$HOME/.config/systemd/user"
if [ -d "$AC_UNIT_DIR" ]; then
    for ac_unit in "$AC_UNIT_DIR"/*.service; do
        [ -f "$ac_unit" ] || continue
        if grep -q 'Generated by axiom-cli Install-AxiomCli.sh' "$ac_unit" 2>/dev/null; then
            AC_AC_UNIT_FILES="$AC_AC_UNIT_FILES $ac_unit"
        fi
    done
fi

if [ "$AC_OWNED" = "0" ] && [ "$AC_ENTRYPOINT_PRESENT" = "0" ] && [ -z "$(printf '%s' "$AC_AC_UNIT_FILES" | tr -d ' ')" ]; then
    if [ "$AC_PURGE" = "1" ]; then
        ac_limitation "--purge was requested but nothing owned by this distribution was found, so no user data was deleted"
    fi
    AC_EV_OUTCOME="not_removed"
    AC_EV_EXIT="$AC_EXIT_NOT_FOUND"
    AC_EV_STATUS="refused"
    AC_EV_MESSAGE="no axiom-cli installation owned by this installer was found at $AC_INSTALL_ROOT"
    ac_refusal "installation-present" "-" "" "" "$AC_EV_MESSAGE"
    ac_finish
fi

# ---------------------------------------------------------------------------
# Removal plan and its digest
# ---------------------------------------------------------------------------

AC_PLAN_INPUT="$AC_TMP/plan-input"
{
    printf 'axiom-cli-linux-uninstall-plan-v1\n'
    printf 'install_root=%s\n' "$AC_INSTALL_ROOT"
    printf 'bin_dir=%s\n' "$AC_BIN_DIR"
    printf 'state_dir=%s\n' "$AC_STATE_DIR"
    printf 'purge=%s\n' "$AC_PURGE"
    printf 'installed_version=%s\n' "$AC_RECORDED_VERSION"
    printf 'installed_plan_digest=%s\n' "$AC_RECORDED_PLAN_DIGEST"
    printf 'entrypoint_present=%s\n' "$AC_ENTRYPOINT_PRESENT"
    printf 'units=%s\n' "$(printf '%s' "$AC_AC_UNIT_FILES" | tr -d ' ')"
} > "$AC_PLAN_INPUT"
AC_PLAN_DIGEST="$(ac_sha256_file "$AC_PLAN_INPUT")"
AC_EV_PLAN_DIGEST="$AC_PLAN_DIGEST"

if [ "$AC_PURGE" = "1" ]; then
    ac_limitation "--purge deletes per-user state; this is the explicit separate approval the distribution guide requires for private state"
else
    ac_preserved "$AC_STATE_DIR" "per-user state is preserved by uninstall; only --purge with an approved digest deletes it"
fi

if [ "$AC_APPLY" != "1" ]; then
    AC_EV_OUTCOME="planned"; AC_EV_EXIT="$AC_EXIT_SUCCESS"; AC_EV_DRY_RUN=1; AC_EV_MUTATED=0
    AC_EV_MESSAGE="uninstall plan computed; nothing was removed"
    ac_log "plan digest $AC_PLAN_DIGEST"
    ac_finish
fi

AC_EV_DRY_RUN=0
AC_EV_APPROVED_DIGEST="$AC_APPROVE_DIGEST"
if [ -z "$AC_APPROVE_DIGEST" ]; then
    ac_refuse "$AC_EXIT_VALIDATION" "--apply requires --approve-digest bound to the plan digest $AC_PLAN_DIGEST" "approval-digest"
fi
if [ "$AC_APPROVE_DIGEST" != "$AC_PLAN_DIGEST" ]; then
    ac_refuse "$AC_EXIT_AUTHORIZATION" "the supplied approval digest does not match the computed removal plan digest" "approval-digest"
fi

AC_EV_TXN="txn-$(date +%s)-$$"
AC_SERVICE_REMOVED=0
AC_SERVICE_DAEMON_RELOADED=0
AC_SERVICE_UNIT_NAME=""
AC_SERVICE_UNIT_PATH=""

# ---------------------------------------------------------------------------
# Remove the generated systemd user units
# ---------------------------------------------------------------------------

for ac_unit in $AC_AC_UNIT_FILES; do
    ac_unit_base="$(basename -- "$ac_unit")"
    if ac_have systemctl && systemctl --user disable "$ac_unit_base" >/dev/null 2>&1; then
        AC_SERVICE_REMOVED=1
    fi
    if rm -f -- "$ac_unit"; then
        AC_SERVICE_REMOVED=1
        AC_SERVICE_UNIT_NAME="$ac_unit_base"
        AC_SERVICE_UNIT_PATH="$ac_unit"
        ac_removed "$ac_unit" "service-registration"
    fi
done
if [ "$AC_SERVICE_REMOVED" = "1" ] && ac_have systemctl; then
    if systemctl --user daemon-reload >/dev/null 2>&1; then
        AC_SERVICE_DAEMON_RELOADED=1
    fi
fi
if [ "$AC_SERVICE_REMOVED" = "1" ]; then
    AC_EV_SERVICE_JSON="$(printf '{"kind":"systemd-user","unit_name":%s,"unit_path":%s,"owner":"axiom-cli","registered":false,"removed":true,"daemon_reloaded":%s,"reason":"the generated user unit was disabled and removed by uninstall"}' \
        "$(ac_json_string_or_null "$AC_SERVICE_UNIT_NAME")" \
        "$(ac_json_string_or_null "$AC_SERVICE_UNIT_PATH")" \
        "$(ac_json_bool "$AC_SERVICE_DAEMON_RELOADED")")"
fi

# ---------------------------------------------------------------------------
# Remove the entrypoint, generations and ownership record
# ---------------------------------------------------------------------------

if [ "$AC_ENTRYPOINT_PRESENT" = "1" ]; then
    if rm -f -- "$AC_BIN_DIR/axiom-cli"; then
        ac_removed "$AC_BIN_DIR/axiom-cli" "executable"
    fi
fi

if [ -d "$AC_INSTALL_ROOT/generations" ]; then
    for ac_gen in "$AC_INSTALL_ROOT"/generations/*; do
        [ -e "$ac_gen" ] || continue
        if rm -rf -- "$ac_gen"; then
            ac_removed "$ac_gen" "generation"
        fi
    done
    rmdir -- "$AC_INSTALL_ROOT/generations" 2>/dev/null || true
fi

if [ -f "$AC_OWNER_MARKER_PATH" ]; then
    if rm -f -- "$AC_OWNER_MARKER_PATH"; then
        ac_removed "$AC_OWNER_MARKER_PATH" "owner-marker"
    fi
fi

if [ -d "$AC_INSTALL_ROOT/.staging" ]; then
    rm -rf -- "$AC_INSTALL_ROOT/.staging"
    ac_removed "$AC_INSTALL_ROOT/.staging" "staging"
fi
if [ -d "$AC_INSTALL_ROOT/.lock" ]; then
    rm -rf -- "$AC_INSTALL_ROOT/.lock"
fi

if [ "$AC_PURGE" = "1" ]; then
    if [ -d "$AC_STATE_DIR" ]; then
        rm -rf -- "$AC_STATE_DIR"
        ac_removed "$AC_STATE_DIR" "state-directory"
    fi
    if [ -d "$AC_INSTALL_ROOT" ]; then
        rmdir -- "$AC_INSTALL_ROOT" 2>/dev/null || true
    fi
    ac_limitation "--purge removed the per-user state directory $AC_STATE_DIR after the approved digest was supplied"
else
    ac_preserved "$AC_STATE_DIR" "per-user state is preserved because --purge was not requested"
    rmdir -- "$AC_INSTALL_ROOT" 2>/dev/null || true
fi

AC_EV_OUTCOME="removed"
AC_EV_EXIT="$AC_EXIT_SUCCESS"
AC_EV_STATUS="ok"
AC_EV_MUTATED=1
AC_EV_MESSAGE="removed the axiom-cli installation owned by this installer from $AC_INSTALL_ROOT"
ac_log "$AC_EV_MESSAGE"
ac_finish
