#!/bin/sh
# Install-AxiomCli.sh - axiom-cli Linux x64 per-user installer.
#
# Target: glibc Linux on x86_64, including the WSL2 lane, which the distribution
# contract (section 8 rule 3) records as `linux-x64` evidence and never as
# Windows evidence.
#
# Properties required by the contract and by axiom-cli/Development.md:
#   * per-user install, never root, never sudo/su/doas, no machine-wide change;
#   * verify every artifact sha256 before use; a mismatch aborts;
#   * atomic swap that keeps the previous generation;
#   * idempotent re-run;
#   * systemd user service used when it is available and degraded explicitly
#     when it is not;
#   * one machine-readable install-result envelope on stdout;
#   * no Bash, no Docker, no Node.js, no shell-string execution.
#
# Usage: Install-AxiomCli.sh --release-set FILE [options]

set -u

AC_EXIT_SUCCESS=0
AC_EXIT_VALIDATION=2
AC_EXIT_NOT_FOUND=3
AC_EXIT_NOT_READY=4
AC_EXIT_AUTHORIZATION=5
AC_EXIT_CONFLICT=6
AC_EXIT_INCOMPATIBLE=9
AC_EXIT_LOCK_UNAVAILABLE=10
AC_EXIT_IO=8

AC_SELF_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
# shellcheck source=AxiomCli.Linux.Common.sh
. "$AC_SELF_DIR/AxiomCli.Linux.Common.sh"

ac_usage() {
    cat <<'USAGE'
axiom-cli Linux x64 per-user installer

USAGE:
    Install-AxiomCli.sh --release-set FILE [OPTIONS]

OPTIONS:
    --release-set FILE     release-set.json describing the artifacts (required)
    --install-root DIR     install root (default $XDG_DATA_HOME/axiom-cli)
    --bin-dir DIR          directory that receives the entrypoint (default $HOME/.local/bin)
    --state-dir DIR        per-user state directory (default $XDG_STATE_HOME/axiom-cli)
    --service MODE         auto | none | systemd-user        (default auto)
    --plan                 compute the plan and stop (default; no mutation)
    --apply                perform the transaction (requires --approve-digest)
    --approve-digest HEX   approval digest bound to the computed plan digest
    --force                replace an entrypoint this installer does not own
    --json                 emit the machine-readable install-result envelope
    --envelope-out FILE    also write the envelope to FILE
    --quiet                suppress progress diagnostics on stderr
    --help                 show this help and exit 0

EXIT CODES (owned by axiom-specs/docs/16-CLI-AND-CONTROL-API.md section 6):
    0 success, 2 validation, 3 not found, 4 not-ready, 5 authorization,
    6 conflict, 8 I/O, 9 incompatible, 10 lock unavailable, 20 partial

NOTES:
    The installer never writes outside the invoking user's own directories and
    never calls sudo. It does not modify PATH; it reports the entry to add.
USAGE
}

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------

AC_RELEASE_SET=""
AC_INSTALL_ROOT=""
AC_BIN_DIR=""
AC_STATE_DIR=""
AC_SERVICE_MODE="auto"
AC_APPLY=0
AC_FORCE=0
AC_QUIET=0
AC_APPROVE_DIGEST=""
AC_ENVELOPE_OUT=""
AC_ARG_ERROR=""

ac_fail_args() {
    AC_ARG_ERROR="$1"
    ac_validate_fail
}

while [ $# -gt 0 ]; do
    case "$1" in
        --release-set)
            [ $# -ge 2 ] || ac_fail_args "--release-set requires a value"
            AC_RELEASE_SET="$2"; shift 2 ;;
        --install-root)
            [ $# -ge 2 ] || ac_fail_args "--install-root requires a value"
            AC_INSTALL_ROOT="$2"; shift 2 ;;
        --bin-dir)
            [ $# -ge 2 ] || ac_fail_args "--bin-dir requires a value"
            AC_BIN_DIR="$2"; shift 2 ;;
        --state-dir)
            [ $# -ge 2 ] || ac_fail_args "--state-dir requires a value"
            AC_STATE_DIR="$2"; shift 2 ;;
        --service)
            [ $# -ge 2 ] || ac_fail_args "--service requires a value"
            AC_SERVICE_MODE="$2"; shift 2 ;;
        --approve-digest)
            [ $# -ge 2 ] || ac_fail_args "--approve-digest requires a value"
            AC_APPROVE_DIGEST="$2"; shift 2 ;;
        --envelope-out)
            [ $# -ge 2 ] || ac_fail_args "--envelope-out requires a value"
            AC_ENVELOPE_OUT="$2"; shift 2 ;;
        --plan)
            shift ;;
        --apply)
            AC_APPLY=1; shift ;;
        --force)
            AC_FORCE=1; shift ;;
        --json)
            shift ;;
        --quiet)
            AC_QUIET=1; shift ;;
        --help|-h)
            ac_usage; exit "$AC_EXIT_SUCCESS" ;;
        *)
            ac_fail_args "unknown option or verb: $1" ;;
    esac
done

case "$AC_SERVICE_MODE" in
    auto|none|systemd-user) ;;
    *) ac_fail_args "--service must be one of auto, none, systemd-user" ;;
esac

if [ -z "$AC_RELEASE_SET" ]; then
    ac_fail_args "--release-set is required"
fi

if [ -n "$AC_APPROVE_DIGEST" ]; then
    case "$AC_APPROVE_DIGEST" in
        *[!0-9a-fA-F]*) ac_fail_args "--approve-digest must be hexadecimal" ;;
    esac
    if [ "${#AC_APPROVE_DIGEST}" -ne 64 ]; then
        ac_fail_args "--approve-digest must be 64 hexadecimal characters"
    fi
fi

# ---------------------------------------------------------------------------
# Path and value helpers
# ---------------------------------------------------------------------------

ac_abs() {
    case "$1" in
        /*) printf '%s' "$1" ;;
        *) printf '%s/%s' "$(pwd)" "$1" ;;
    esac
}

ac_json_obj_field() {
    # $1 = object text, $2 = key
    printf '%s' "$1" | sed -n "s/.*\"$2\":\"\([^\"]*\)\".*/\1/p"
}

ac_json_obj_number() {
    printf '%s' "$1" | sed -n "s/.*\"$2\":\([0-9][0-9]*\).*/\1/p"
}

ac_is_sha256() {
    case "$1" in
        *[!0-9a-f]*) return 1 ;;
    esac
    [ "${#1}" -eq 64 ] || return 1
    return 0
}

# Numeric comparison of the version core, ignoring a `-suffix`. Prints why it
# cannot decide as `undecided` so the caller records a limitation instead of
# guessing.
ac_version_numeric_lt() {
    ac_va="${1%%-*}"
    ac_vb="${2%%-*}"
    ac_i=1
    while [ "$ac_i" -le 3 ]; do
        ac_x="$(printf '%s' "$ac_va" | cut -d. -f"$ac_i")"
        ac_y="$(printf '%s' "$ac_vb" | cut -d. -f"$ac_i")"
        [ -n "$ac_x" ] || ac_x=0
        [ -n "$ac_y" ] || ac_y=0
        case "$ac_x$ac_y" in
            *[!0-9]*) printf 'undecided'; return 0 ;;
        esac
        if [ "$ac_x" -lt "$ac_y" ]; then printf '1'; return 0; fi
        if [ "$ac_x" -gt "$ac_y" ]; then printf '0'; return 0; fi
        ac_i=$((ac_i + 1))
    done
    printf '0'
}

# ---------------------------------------------------------------------------
# Envelope assembly helpers
# ---------------------------------------------------------------------------

AC_TMP="$(mktemp -d 2>/dev/null || printf '%s/axiom-cli-install-%s' "${TMPDIR:-/tmp}" "$$")"
[ -d "$AC_TMP" ] || mkdir -p -- "$AC_TMP"
AC_LIST_ARTIFACTS="$AC_TMP/list-artifacts"
AC_LIST_UNVERIFIED="$AC_TMP/list-unverified"
AC_LIST_COMPONENTS="$AC_TMP/list-components"
AC_LIST_REFUSALS="$AC_TMP/list-refusals"
AC_LIST_PRESERVED="$AC_TMP/list-preserved"
AC_LIST_REMOVED="$AC_TMP/list-removed"
AC_LIST_LIMITATIONS="$AC_TMP/list-limitations"
: > "$AC_LIST_ARTIFACTS"
: > "$AC_LIST_UNVERIFIED"
: > "$AC_LIST_COMPONENTS"
: > "$AC_LIST_REFUSALS"
: > "$AC_LIST_PRESERVED"
: > "$AC_LIST_REMOVED"
: > "$AC_LIST_LIMITATIONS"

ac_list_add() {
    printf '%s\n' "$2" >> "$1"
}

ac_list_json() {
    ac_json_acc="["
    ac_json_first=1
    while IFS= read -r ac_json_line; do
        [ -n "$ac_json_line" ] || continue
        if [ "$ac_json_first" = "0" ]; then
            ac_json_acc="$ac_json_acc,"
        fi
        ac_json_acc="$ac_json_acc$ac_json_line"
        ac_json_first=0
    done < "$1"
    printf '%s]' "$ac_json_acc"
}

ac_limitation() {
    ac_list_add "$AC_LIST_LIMITATIONS" "$(ac_json_string "$1")"
}

ac_refusal() {
    # $1 check, $2 artifact, $3 expected, $4 actual, $5 reason
    ac_list_add "$AC_LIST_REFUSALS" "$(printf '{"check":%s,"artifact":%s,"expected_sha256":%s,"actual_sha256":%s,"reason":%s}' \
        "$(ac_json_string "$1")" \
        "$(ac_json_string "$2")" \
        "$(ac_json_string_or_null "$3")" \
        "$(ac_json_string_or_null "$4")" \
        "$(ac_json_string "$5")")"
}

ac_removed() {
    ac_list_add "$AC_LIST_REMOVED" "$(printf '{"path":%s,"kind":%s}' \
        "$(ac_json_string "$1")" "$(ac_json_string "$2")")"
}

ac_preserved() {
    ac_list_add "$AC_LIST_PRESERVED" "$(printf '{"path":%s,"reason":%s}' \
        "$(ac_json_string "$1")" "$(ac_json_string "$2")")"
}

ac_unverified() {
    ac_list_add "$AC_LIST_UNVERIFIED" "$(printf '{"name":%s,"component":%s,"reason":%s}' \
        "$(ac_json_string "$1")" "$(ac_json_string "$2")" "$(ac_json_string "$3")")"
}

ac_component() {
    ac_list_add "$AC_LIST_COMPONENTS" "$(printf '{"component":%s,"installed_version":%s,"artifact_sha256":%s,"version_source":%s}' \
        "$(ac_json_string "$1")" \
        "$(ac_json_string_or_null "$2")" \
        "$(ac_json_string_or_null "$3")" \
        "$(ac_json_string "$4")")"
}

# ---------------------------------------------------------------------------
# Envelope emission. Values are collected in AC_* variables, then written once.
# ---------------------------------------------------------------------------

AC_EV_OPERATION="install"
AC_EV_OUTCOME="planned"
AC_EV_EXIT="$AC_EXIT_SUCCESS"
AC_EV_STATUS="ok"
AC_EV_MESSAGE=""
AC_EV_RETRYABLE=0
AC_EV_INSTALL_ROOT=""
AC_EV_BIN_DIR=""
AC_EV_DRY_RUN=1
AC_EV_MUTATED=0
AC_EV_PLAN_DIGEST=""
AC_EV_APPROVED_DIGEST=""
AC_EV_TXN=""
AC_EV_RECOVERED=0
AC_EV_RELEASE_SET_JSON="null"
AC_EV_PATH_RULE_JSON='{"scope":"none","file":null,"entry":"","applied":false,"machine_wide_change":false,"previous_present":false}'
AC_EV_SERVICE_JSON='{"kind":"none","unit_name":null,"unit_path":null,"owner":null,"registered":false,"removed":false,"daemon_reloaded":false,"reason":"no service registration was attempted"}'

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
    ac_put install_root "$(ac_json_string_or_null "$AC_EV_INSTALL_ROOT")"
    ac_put bin_dir "$(ac_json_string_or_null "$AC_EV_BIN_DIR")"
    ac_put dry_run "$(ac_json_bool "$AC_EV_DRY_RUN")"
    ac_put mutated "$(ac_json_bool "$AC_EV_MUTATED")"
    ac_put plan_digest "$(ac_json_string_or_null "$AC_EV_PLAN_DIGEST")"
    ac_put approved_digest "$(ac_json_string_or_null "$AC_EV_APPROVED_DIGEST")"
    ac_put transaction_id "$(ac_json_string_or_null "$AC_EV_TXN")"
    ac_put interrupted_install_recovered "$(ac_json_bool "$AC_EV_RECOVERED")"
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
    AC_EV_OUTCOME="refused"
    AC_EV_EXIT="$AC_EXIT_VALIDATION"
    AC_EV_STATUS="refused"
    AC_EV_MESSAGE="${AC_ARG_ERROR:-invalid arguments}"
    ac_refusal "arguments" "-" "" "" "$AC_EV_MESSAGE"
    ac_finish
}

ac_refuse() {
    # $1 exit, $2 message, $3 refusal check
    AC_EV_OUTCOME="refused"
    AC_EV_EXIT="$1"
    AC_EV_STATUS="refused"
    AC_EV_MESSAGE="$2"
    ac_refusal "$3" "-" "" "" "$2"
    ac_finish
}

# ---------------------------------------------------------------------------
# 1. Resolve paths
# ---------------------------------------------------------------------------

AC_DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
AC_STATE_HOME="${XDG_STATE_HOME:-$HOME/.local/state}"
[ -n "$AC_INSTALL_ROOT" ] || AC_INSTALL_ROOT="$AC_DATA_HOME/axiom-cli"
[ -n "$AC_BIN_DIR" ] || AC_BIN_DIR="$HOME/.local/bin"
[ -n "$AC_STATE_DIR" ] || AC_STATE_DIR="$AC_STATE_HOME/axiom-cli"
AC_INSTALL_ROOT="$(ac_abs "$AC_INSTALL_ROOT")"
AC_BIN_DIR="$(ac_abs "$AC_BIN_DIR")"
AC_STATE_DIR="$(ac_abs "$AC_STATE_DIR")"
AC_EV_INSTALL_ROOT="$AC_INSTALL_ROOT"
AC_EV_BIN_DIR="$AC_BIN_DIR"

AC_EV_PATH_RULE_JSON="$(printf '{"scope":"none","file":null,"entry":%s,"applied":false,"machine_wide_change":false,"previous_present":false}' "$(ac_json_string "$AC_BIN_DIR")")"

if ! ac_have sha256sum; then
    ac_refuse "$AC_EXIT_NOT_READY" \
        "coreutils sha256sum is not available, so no artifact digest can be verified and the transaction was not started" \
        "sha256sum-available"
fi

if ! ac_have cp || ! ac_have mkdir || ! ac_have mv || ! ac_have rm; then
    ac_refuse "$AC_EXIT_NOT_READY" \
        "coreutils mkdir/cp/mv/rm are required to stage the install and were not all found" \
        "coreutils-available"
fi

# ---------------------------------------------------------------------------
# 2. Read and validate the release set
# ---------------------------------------------------------------------------

AC_RELEASE_SET="$(ac_abs "$AC_RELEASE_SET")"
if [ ! -f "$AC_RELEASE_SET" ]; then
    ac_refuse "$AC_EXIT_NOT_FOUND" "release set not found: $AC_RELEASE_SET" "release-set-present"
fi
if [ ! -r "$AC_RELEASE_SET" ]; then
    ac_refuse "$AC_EXIT_IO" "release set is not readable: $AC_RELEASE_SET" "release-set-readable"
fi

AC_RELEASE_SET_DIR="$(dirname -- "$AC_RELEASE_SET")"
AC_RS_SHA="$(ac_sha256_file "$AC_RELEASE_SET")"
AC_RS_VERSION="$(sed -n 's/^[[:space:]]*"release_version":[[:space:]]*"\(.*\)",\{0,1\}[[:space:]]*$/\1/p' "$AC_RELEASE_SET" | head -n 1)"
AC_RS_PLATFORM="$(sed -n 's/^[[:space:]]*"platform":[[:space:]]*"\(.*\)",\{0,1\}[[:space:]]*$/\1/p' "$AC_RELEASE_SET" | head -n 1)"
AC_RS_TRIPLE="$(sed -n 's/^[[:space:]]*"target_triple":[[:space:]]*"\(.*\)",\{0,1\}[[:space:]]*$/\1/p' "$AC_RELEASE_SET" | head -n 1)"
AC_RS_LIBC="$(sed -n 's/^[[:space:]]*"libc":[[:space:]]*"\(.*\)",\{0,1\}[[:space:]]*$/\1/p' "$AC_RELEASE_SET" | head -n 1)"

if [ -z "$AC_RS_VERSION" ] || [ -z "$AC_RS_PLATFORM" ]; then
    ac_refuse "$AC_EXIT_INCOMPATIBLE" \
        "the release set does not carry the canonical line-oriented shape (release_version/platform missing), so it was refused instead of being misread" \
        "release-set-shape"
fi
if [ "$AC_RS_PLATFORM" != "linux-x64" ]; then
    ac_refuse "$AC_EXIT_INCOMPATIBLE" \
        "release set targets platform '$AC_RS_PLATFORM' but this installer serves linux-x64" \
        "release-set-platform"
fi

AC_ARTIFACTS_PARSED="$AC_TMP/artifacts-parsed"
: > "$AC_ARTIFACTS_PARSED"
AC_USER_SERVICE_LINE="$(sed -n 's/^[[:space:]]*"user_service":[[:space:]]*\({.*}\),\{0,1\}[[:space:]]*$/\1/p' "$AC_RELEASE_SET" | head -n 1)"

AC_SEEN_ARTIFACT=0
while IFS= read -r ac_art_line; do
    [ -n "$ac_art_line" ] || continue
    AC_SEEN_ARTIFACT=1
    ac_a_name="$(ac_json_obj_field "$ac_art_line" name)"
    ac_a_component="$(ac_json_obj_field "$ac_art_line" component)"
    ac_a_version="$(ac_json_obj_field "$ac_art_line" version)"
    ac_a_arch="$(ac_json_obj_field "$ac_art_line" arch)"
    ac_a_sha="$(ac_json_obj_field "$ac_art_line" sha256)"
    ac_a_size="$(ac_json_obj_number "$ac_art_line" size_bytes)"
    ac_a_archive="$(ac_json_obj_field "$ac_art_line" archive)"
    ac_a_kind="$(ac_json_obj_field "$ac_art_line" kind)"
    if [ -z "$ac_a_name" ] || [ -z "$ac_a_component" ] || [ -z "$ac_a_version" ] || [ -z "$ac_a_arch" ] || [ -z "$ac_a_archive" ]; then
        ac_refuse "$AC_EXIT_INCOMPATIBLE" \
            "an artifact entry is missing a required field (name/component/version/arch/archive)" \
            "artifact-shape"
    fi
    if ! ac_is_sha256 "$ac_a_sha"; then
        ac_refuse "$AC_EXIT_INCOMPATIBLE" \
            "artifact '$ac_a_name' does not declare a lowercase 64-character sha256" \
            "artifact-digest-shape"
    fi
    case "$ac_a_archive" in
        /*|*..*)
            ac_refuse "$AC_EXIT_INCOMPATIBLE" \
                "artifact '$ac_a_name' declares an archive path that escapes the release set directory: $ac_a_archive" \
                "artifact-archive-relative" ;;
    esac
    if [ ! -f "$AC_RELEASE_SET_DIR/$ac_a_archive" ]; then
        ac_refuse "$AC_EXIT_NOT_FOUND" \
            "artifact '$ac_a_name' is declared but its file is absent: $ac_a_archive" \
            "artifact-present"
    fi
    ac_a_actual_sha="$(ac_sha256_file "$AC_RELEASE_SET_DIR/$ac_a_archive")"
    if [ "$ac_a_actual_sha" != "$ac_a_sha" ]; then
        ac_refusal "artifact-digest" "$ac_a_name" "$ac_a_sha" "$ac_a_actual_sha" \
            "the declared sha256 does not match the artifact bytes; the transaction was aborted before anything was staged"
        AC_EV_OUTCOME="refused"
        AC_EV_EXIT="$AC_EXIT_INCOMPATIBLE"
        AC_EV_STATUS="refused"
        AC_EV_MESSAGE="artifact '$ac_a_name' failed digest verification"
        ac_finish
    fi
    ac_a_actual_size="$(wc -c < "$AC_RELEASE_SET_DIR/$ac_a_archive" | tr -d ' ')"
    if [ -n "$ac_a_size" ] && [ "$ac_a_size" != "$ac_a_actual_size" ]; then
        ac_refusal "artifact-size" "$ac_a_name" "" "" \
            "the declared size_bytes ($ac_a_size) does not match the artifact byte length ($ac_a_actual_size)"
        AC_EV_OUTCOME="refused"
        AC_EV_EXIT="$AC_EXIT_INCOMPATIBLE"
        AC_EV_STATUS="refused"
        AC_EV_MESSAGE="artifact '$ac_a_name' failed size verification"
        ac_finish
    fi
    printf '%s|%s|%s|%s|%s|%s|%s|%s\n' \
        "$ac_a_name" "$ac_a_component" "$ac_a_version" "$ac_a_arch" "$ac_a_sha" "$ac_a_actual_size" "$ac_a_archive" "$ac_a_kind" \
        >> "$AC_ARTIFACTS_PARSED"
done <<EOF
$(sed -n 's/^[[:space:]]*\({"name".*}\),\{0,1\}[[:space:]]*$/\1/p' "$AC_RELEASE_SET")
EOF

if [ "$AC_SEEN_ARTIFACT" != "1" ]; then
    ac_refuse "$AC_EXIT_INCOMPATIBLE" \
        "the release set declares no artifact, so there is nothing to install" \
        "release-set-non-empty"
fi

# ---------------------------------------------------------------------------
# 3. Plan digest (binds install root, bin dir, service decision and artifacts)
# ---------------------------------------------------------------------------

AC_PLAN_INPUT="$AC_TMP/plan-input"
{
    printf 'axiom-cli-linux-plan-v1\n'
    printf 'platform=linux-x64\n'
    printf 'install_root=%s\n' "$AC_INSTALL_ROOT"
    printf 'bin_dir=%s\n' "$AC_BIN_DIR"
    printf 'state_dir=%s\n' "$AC_STATE_DIR"
    printf 'service_mode=%s\n' "$AC_SERVICE_MODE"
    printf 'release_version=%s\n' "$AC_RS_VERSION"
    printf 'release_set_sha256=%s\n' "$AC_RS_SHA"
    while IFS= read -r ac_plan_line; do
        [ -n "$ac_plan_line" ] || continue
        printf 'artifact=%s\n' "$ac_plan_line"
    done < "$AC_ARTIFACTS_PARSED"
} > "$AC_PLAN_INPUT"
AC_PLAN_DIGEST="$(ac_sha256_file "$AC_PLAN_INPUT")"
AC_EV_PLAN_DIGEST="$AC_PLAN_DIGEST"
AC_EV_RELEASE_SET_JSON="$(printf '{"path":%s,"sha256":%s,"release_version":%s}' \
    "$(ac_json_string "$AC_RELEASE_SET")" "$(ac_json_string "$AC_RS_SHA")" "$(ac_json_string "$AC_RS_VERSION")")"

ac_render_artifacts() {
    # $1 = directory the artifacts were installed into ("" for a plan)
    : > "$AC_LIST_ARTIFACTS"
    while IFS= read -r ac_r_line; do
        [ -n "$ac_r_line" ] || continue
        ac_r_name="$(printf '%s' "$ac_r_line" | cut -d'|' -f1)"
        ac_r_component="$(printf '%s' "$ac_r_line" | cut -d'|' -f2)"
        ac_r_version="$(printf '%s' "$ac_r_line" | cut -d'|' -f3)"
        ac_r_arch="$(printf '%s' "$ac_r_line" | cut -d'|' -f4)"
        ac_r_sha="$(printf '%s' "$ac_r_line" | cut -d'|' -f5)"
        ac_r_size="$(printf '%s' "$ac_r_line" | cut -d'|' -f6)"
        ac_r_archive="$(printf '%s' "$ac_r_line" | cut -d'|' -f7)"
        if [ -n "$1" ]; then
            ac_r_path="$(ac_json_string "$1/$ac_r_name")"
        else
            ac_r_path="null"
        fi
        ac_list_add "$AC_LIST_ARTIFACTS" "$(printf '{"name":%s,"component":%s,"version":%s,"arch":%s,"sha256":%s,"size_bytes":%s,"verified":true,"installed_path":%s,"source":%s}' \
            "$(ac_json_string "$ac_r_name")" \
            "$(ac_json_string "$ac_r_component")" \
            "$(ac_json_string "$ac_r_version")" \
            "$(ac_json_string "$ac_r_arch")" \
            "$(ac_json_string "$ac_r_sha")" \
            "$ac_r_size" \
            "$ac_r_path" \
            "$(ac_json_string "$ac_r_archive")")"
    done < "$AC_ARTIFACTS_PARSED"
}

ac_render_components() {
    : > "$AC_LIST_COMPONENTS"
    while IFS= read -r ac_c_line; do
        [ -n "$ac_c_line" ] || continue
        ac_c_component="$(printf '%s' "$ac_c_line" | cut -d'|' -f2)"
        ac_c_version="$(printf '%s' "$ac_c_line" | cut -d'|' -f3)"
        ac_c_sha="$(printf '%s' "$ac_c_line" | cut -d'|' -f5)"
        ac_component "$ac_c_component" "$ac_c_version" "$ac_c_sha" "release-set.json (declared by this release set)"
    done < "$AC_ARTIFACTS_PARSED"
}

ac_render_unverified() {
    : > "$AC_LIST_UNVERIFIED"
    for ac_u_component in axiom-graphd axiom-mcp skills; do
        ac_u_present=0
        while IFS= read -r ac_u_line; do
            [ -n "$ac_u_line" ] || continue
            ac_u_component_of_line="$(printf '%s' "$ac_u_line" | cut -d'|' -f2)"
            if [ "$ac_u_component_of_line" = "$ac_u_component" ]; then
                ac_u_present=1
            fi
        done < "$AC_ARTIFACTS_PARSED"
        if [ "$ac_u_present" = "0" ]; then
            ac_unverified "$ac_u_component" "$ac_u_component" \
                "the distribution contract requires this component in a complete installation, but this release set carries no pinned artifact for it, so it was not installed and nothing about its state is claimed"
        fi
    done
}

ac_render_artifacts ""
ac_render_components
ac_render_unverified
ac_limitation "entrypoint directory $AC_BIN_DIR is reported but never added to PATH by this installer (no hidden PATH mutation); add it yourself if it is not already on PATH"

# ---------------------------------------------------------------------------
# 4. Plan mode stops here without mutating anything
# ---------------------------------------------------------------------------

if [ "$AC_APPLY" != "1" ]; then
    AC_EV_OUTCOME="planned"
    AC_EV_EXIT="$AC_EXIT_SUCCESS"
    AC_EV_DRY_RUN=1
    AC_EV_MUTATED=0
    AC_EV_MESSAGE="install plan computed; nothing was created, copied or registered"
    ac_log "plan digest $AC_PLAN_DIGEST"
    ac_finish
fi

# ---------------------------------------------------------------------------
# 5. Apply: approval, lock, recovery
# ---------------------------------------------------------------------------

AC_EV_DRY_RUN=0
AC_EV_APPROVED_DIGEST="$AC_APPROVE_DIGEST"

if [ -z "$AC_APPROVE_DIGEST" ]; then
    ac_refuse "$AC_EXIT_VALIDATION" \
        "--apply requires --approve-digest bound to the plan digest $AC_PLAN_DIGEST" \
        "approval-digest"
fi
if [ "$AC_APPROVE_DIGEST" != "$AC_PLAN_DIGEST" ]; then
    ac_refuse "$AC_EXIT_AUTHORIZATION" \
        "the supplied approval digest does not match the computed plan digest, so the approval does not cover this plan" \
        "approval-digest"
fi

AC_EV_TXN="txn-$(date +%s)-$$"

# Entrypoint ownership guard that runs BEFORE any filesystem mutation. When the
# install root does not yet carry this installer's ownership record, a foreign
# file at the entrypoint path is a conflict and is refused without creating the
# install root (found by L07). The marker-based guard further below still covers
# the case where an install root from a previous generation already exists.
if [ -e "$AC_BIN_DIR/axiom-cli" ] && [ "$AC_FORCE" != "1" ] \
    && [ ! -f "$AC_INSTALL_ROOT/$AC_OWNER_MARKER" ]; then
    ac_refuse "$AC_EXIT_CONFLICT" \
        "an entrypoint already exists at $AC_BIN_DIR/axiom-cli and is not owned by this installer; pass --force to replace it" \
        "unowned-entrypoint"
fi

if ! mkdir -p -- "$AC_INSTALL_ROOT" 2>/dev/null; then
    ac_refuse "$AC_EXIT_IO" "cannot create the install root $AC_INSTALL_ROOT" "install-root-writable"
fi
if ! ac_acquire_lock; then
    AC_EV_RETRYABLE=1
    ac_refuse "$AC_EXIT_LOCK_UNAVAILABLE" \
        "another install transaction holds the lock at $AC_INSTALL_ROOT/.lock" \
        "transaction-lock"
fi

AC_RECOVERED=0
if [ -d "$AC_INSTALL_ROOT/.staging" ]; then
    if [ -n "$(ls -A "$AC_INSTALL_ROOT/.staging" 2>/dev/null)" ]; then
        AC_RECOVERED=1
        rm -rf -- "$AC_INSTALL_ROOT/.staging"
        ac_removed "$AC_INSTALL_ROOT/.staging" "staging"
        ac_log "recovered an interrupted install: the abandoned staging directory was removed"
    else
        rmdir -- "$AC_INSTALL_ROOT/.staging" 2>/dev/null || true
    fi
fi
AC_EV_RECOVERED="$AC_RECOVERED"

# Ownership marker by previous generations.
AC_INSTALLED_VERSION=""
AC_INSTALLED_PLAN_DIGEST=""
AC_INSTALLED_BIN_DIR=""
AC_INSTALLED_ROOT=""
if [ -f "$AC_INSTALL_ROOT/$AC_OWNER_MARKER" ]; then
    AC_INSTALLED_VERSION="$(sed -n 's/^version=//p' "$AC_INSTALL_ROOT/$AC_OWNER_MARKER" | head -n 1)"
    AC_INSTALLED_PLAN_DIGEST="$(sed -n 's/^plan_digest=//p' "$AC_INSTALL_ROOT/$AC_OWNER_MARKER" | head -n 1)"
    AC_INSTALLED_BIN_DIR="$(sed -n 's/^bin_dir=//p' "$AC_INSTALL_ROOT/$AC_OWNER_MARKER" | head -n 1)"
    AC_INSTALLED_ROOT="$(sed -n 's/^install_root=//p' "$AC_INSTALL_ROOT/$AC_OWNER_MARKER" | head -n 1)"
fi

# Idempotent re-run: same plan digest already installed.
if [ -n "$AC_INSTALLED_PLAN_DIGEST" ] && [ "$AC_INSTALLED_PLAN_DIGEST" = "$AC_PLAN_DIGEST" ] \
    && [ "$AC_INSTALLED_BIN_DIR" = "$AC_BIN_DIR" ] && [ -f "$AC_BIN_DIR/axiom-cli" ]; then
    ac_release_lock
    AC_EV_OUTCOME="already_installed"
    AC_EV_EXIT="$AC_EXIT_SUCCESS"
    AC_EV_MUTATED=0
    AC_EV_MESSAGE="the requested release is already installed at $AC_INSTALL_ROOT; nothing was changed"
    ac_log "$AC_EV_MESSAGE"
    ac_finish
fi

# Downgrade guard.
if [ -n "$AC_INSTALLED_VERSION" ]; then
    ac_downgrade="$(ac_version_numeric_lt "$AC_RS_VERSION" "$AC_INSTALLED_VERSION")"
    if [ "$ac_downgrade" = "1" ]; then
        ac_release_lock
        ac_refuse "$AC_EXIT_INCOMPATIBLE" \
            "the release set version $AC_RS_VERSION is older than the installed version $AC_INSTALLED_VERSION; a downgrade is not performed by install" \
            "no-downgrade"
    fi
    if [ "$ac_downgrade" = "undecided" ]; then
        ac_limitation "the installed version '$AC_INSTALLED_VERSION' and the release version '$AC_RS_VERSION' could not be compared numerically, so no downgrade decision was enforced"
    fi
fi

# Entrypoint ownership guard.
if [ -e "$AC_BIN_DIR/axiom-cli" ] && [ "$AC_FORCE" != "1" ]; then
    ac_owned=0
    if [ -n "$AC_INSTALLED_ROOT" ] && [ "$AC_INSTALLED_ROOT" = "$AC_INSTALL_ROOT" ] \
        && [ "$AC_INSTALLED_BIN_DIR" = "$AC_BIN_DIR" ]; then
        ac_owned=1
    fi
    if [ "$ac_owned" != "1" ]; then
        ac_release_lock
        ac_refuse "$AC_EXIT_CONFLICT" \
            "an entrypoint already exists at $AC_BIN_DIR/axiom-cli and is not owned by this installer; pass --force to replace it" \
            "unowned-entrypoint"
    fi
fi

# ---------------------------------------------------------------------------
# 6. Copy the release into a generation, then swap
# ---------------------------------------------------------------------------

AC_GEN_NAME="$AC_RS_VERSION-$(printf '%s' "$AC_PLAN_DIGEST" | cut -c1-12)"
AC_GEN_DIR="$AC_INSTALL_ROOT/generations/$AC_GEN_NAME"
AC_STAGING="$AC_INSTALL_ROOT/.staging"

if ! mkdir -p -- "$AC_STAGING"; then
    ac_release_lock
    ac_refuse "$AC_EXIT_IO" "cannot create the staging directory $AC_STAGING" "staging-writable"
fi

while IFS= read -r ac_s_line; do
    [ -n "$ac_s_line" ] || continue
    ac_s_name="$(printf '%s' "$ac_s_line" | cut -d'|' -f1)"
    ac_s_sha="$(printf '%s' "$ac_s_line" | cut -d'|' -f5)"
    ac_s_archive="$(printf '%s' "$ac_s_line" | cut -d'|' -f7)"
    ac_s_kind="$(printf '%s' "$ac_s_line" | cut -d'|' -f8)"
    if ! cp -- "$AC_RELEASE_SET_DIR/$ac_s_archive" "$AC_STAGING/$ac_s_name"; then
        rm -rf -- "$AC_STAGING"
        ac_release_lock
        ac_refuse "$AC_EXIT_IO" "cannot stage artifact $ac_s_name" "staging-copy"
    fi
    ac_s_staged_sha="$(ac_sha256_file "$AC_STAGING/$ac_s_name")"
    if [ "$ac_s_staged_sha" != "$ac_s_sha" ]; then
        rm -rf -- "$AC_STAGING"
        ac_release_lock
        ac_refusal "staged-digest" "$ac_s_name" "$ac_s_sha" "$ac_s_staged_sha" \
            "the staged copy does not match the declared digest, so nothing was swapped"
        AC_EV_OUTCOME="refused"
        AC_EV_EXIT="$AC_EXIT_INCOMPATIBLE"
        AC_EV_STATUS="refused"
        AC_EV_MESSAGE="artifact '$ac_s_name' changed during staging"
        ac_finish
    fi
    if [ "$ac_s_kind" = "executable" ]; then
        chmod 0755 "$AC_STAGING/$ac_s_name" 2>/dev/null || true
    fi
done < "$AC_ARTIFACTS_PARSED"

mkdir -p -- "$AC_INSTALL_ROOT/generations"
if [ -e "$AC_GEN_DIR" ]; then
    rm -rf -- "$AC_GEN_DIR"
fi
if ! mv -- "$AC_STAGING" "$AC_GEN_DIR"; then
    rm -rf -- "$AC_STAGING"
    ac_release_lock
    ac_refuse "$AC_EXIT_IO" "cannot move the staged release into generation $AC_GEN_DIR" "generation-swap"
fi

# Entrypoint: a real copy, never a symlink (native behavior must not depend on
# symlinks), written to a temporary name and renamed into place.
mkdir -p -- "$AC_BIN_DIR"
AC_ENTRYPOINT_SOURCE="$AC_GEN_DIR/axiom-cli"
if [ ! -f "$AC_ENTRYPOINT_SOURCE" ]; then
    for ac_cand in "$AC_GEN_DIR"/*; do
        if [ -f "$ac_cand" ]; then
            AC_ENTRYPOINT_SOURCE="$ac_cand"
            break
        fi
    done
fi
if [ ! -f "$AC_ENTRYPOINT_SOURCE" ]; then
    rm -rf -- "$AC_GEN_DIR"
    ac_release_lock
    ac_refuse "$AC_EXIT_IO" "the release set carries no file to install as the axiom-cli entrypoint" "entrypoint-source"
fi
if ! cp -- "$AC_ENTRYPOINT_SOURCE" "$AC_BIN_DIR/.axiom-cli.$AC_EV_TXN"; then
    rm -rf -- "$AC_GEN_DIR"
    ac_release_lock
    ac_refuse "$AC_EXIT_IO" "cannot write the entrypoint into $AC_BIN_DIR" "entrypoint-writable"
fi
chmod 0755 "$AC_BIN_DIR/.axiom-cli.$AC_EV_TXN" 2>/dev/null || true
if ! mv -f -- "$AC_BIN_DIR/.axiom-cli.$AC_EV_TXN" "$AC_BIN_DIR/axiom-cli"; then
    rm -f -- "$AC_BIN_DIR/.axiom-cli.$AC_EV_TXN"
    rm -rf -- "$AC_GEN_DIR"
    ac_release_lock
    ac_refuse "$AC_EXIT_IO" "cannot atomically place the entrypoint at $AC_BIN_DIR/axiom-cli" "entrypoint-swap"
fi
AC_ENTRYPOINT_SHA="$(ac_sha256_file "$AC_BIN_DIR/axiom-cli")"

# ---------------------------------------------------------------------------
# 7. systemd user service: used when available, degraded explicitly when not
# ---------------------------------------------------------------------------

AC_SERVICE_JSON_KIND="none"
AC_SERVICE_UNIT_NAME=""
AC_SERVICE_UNIT_PATH=""
AC_SERVICE_REGISTERED=0
AC_SERVICE_REMOVED=0
AC_SERVICE_DAEMON_RELOADED=0

if [ "$AC_SERVICE_MODE" = "none" ]; then
    AC_SERVICE_REASON="service registration was disabled by --service none"
elif [ -z "$AC_USER_SERVICE_LINE" ]; then
    AC_SERVICE_REASON="this release set declares no user service, so none was registered"
else
    AC_SVC_UNIT_NAME="$(ac_json_obj_field "$AC_USER_SERVICE_LINE" unit_name)"
    AC_SVC_DESCRIPTION="$(ac_json_obj_field "$AC_USER_SERVICE_LINE" description)"
    AC_SVC_EXEC_START="$(ac_json_obj_field "$AC_USER_SERVICE_LINE" exec_start)"
    AC_SVC_TYPE="$(ac_json_obj_field "$AC_USER_SERVICE_LINE" type)"
    AC_SVC_OWNER="$(ac_json_obj_field "$AC_USER_SERVICE_LINE" owner)"
    if [ -z "$AC_SVC_UNIT_NAME" ] || [ -z "$AC_SVC_EXEC_START" ]; then
        ac_limitation "the release set declares a user service without unit_name/exec_start, so no service was registered"
        AC_SERVICE_REASON="the declared user service is incomplete (unit_name/exec_start missing)"
    elif ! ac_systemd_user_available; then
        AC_SERVICE_REASON="$(ac_systemd_user_reason)"
        if [ "$AC_SERVICE_MODE" = "systemd-user" ]; then
            ac_limitation "$AC_SERVICE_REASON; --service systemd-user was requested explicitly, so the install is reported not-ready instead of silently skipping the registration"
            rm -f -- "$AC_BIN_DIR/axiom-cli"
            rm -rf -- "$AC_GEN_DIR"
            ac_release_lock
            ac_refuse "$AC_EXIT_NOT_READY" \
                "the requested systemd user service cannot be registered: $AC_SERVICE_REASON" \
                "systemd-user-available"
        fi
        ac_limitation "$AC_SERVICE_REASON; the service step degraded explicitly and the rest of the install proceeded"
    else
        AC_SVC_UNIT_DIR="$HOME/.config/systemd/user"
        mkdir -p -- "$AC_SVC_UNIT_DIR"
        AC_SVC_UNIT_PATH="$AC_SVC_UNIT_DIR/$AC_SVC_UNIT_NAME"
        {
            printf '# Generated by axiom-cli Install-AxiomCli.sh.\n'
            printf '# Owned by %s; removed by Uninstall-AxiomCli.sh.\n\n' "${AC_SVC_OWNER:-axiom-cli}"
            printf '[Unit]\n'
            printf 'Description=%s\n' "${AC_SVC_DESCRIPTION:-Axiom CLI component}"
            printf '\n[Service]\n'
            printf 'Type=%s\n' "${AC_SVC_TYPE:-oneshot}"
            printf 'ExecStart=%s\n' "$AC_SVC_EXEC_START"
            printf '\n[Install]\n'
            printf 'WantedBy=default.target\n'
        } > "$AC_SVC_UNIT_PATH"
        if systemctl --user daemon-reload >/dev/null 2>&1; then
            AC_SERVICE_DAEMON_RELOADED=1
        fi
        if systemctl --user enable "$AC_SVC_UNIT_NAME" >/dev/null 2>&1; then
            AC_SERVICE_REGISTERED=1
            AC_SERVICE_JSON_KIND="systemd-user"
            AC_SERVICE_UNIT_NAME="$AC_SVC_UNIT_NAME"
            AC_SERVICE_UNIT_PATH="$AC_SVC_UNIT_PATH"
            AC_SERVICE_REASON="the release set declared a user service and a systemd user manager was reachable, so the unit was written, daemon-reloaded and enabled"
        else
            AC_SERVICE_REASON="a systemd user manager answered but the unit could not be enabled; the unit file was written and the enable step reported failure"
            ac_limitation "$AC_SERVICE_REASON"
            AC_SERVICE_JSON_KIND="none"
        fi
    fi
fi

AC_SVC_OWNER_VALUE="$(ac_json_string_or_null "${AC_SVC_OWNER:-}")"
if [ "$AC_SERVICE_JSON_KIND" = "none" ]; then
    AC_SVC_OWNER_VALUE="null"
fi
AC_EV_SERVICE_JSON="$(printf '{"kind":%s,"unit_name":%s,"unit_path":%s,"owner":%s,"registered":%s,"removed":%s,"daemon_reloaded":%s,"reason":%s}' \
    "$(ac_json_string "$AC_SERVICE_JSON_KIND")" \
    "$(ac_json_string_or_null "$AC_SERVICE_UNIT_NAME")" \
    "$(ac_json_string_or_null "$AC_SERVICE_UNIT_PATH")" \
    "$AC_SVC_OWNER_VALUE" \
    "$(ac_json_bool "$AC_SERVICE_REGISTERED")" \
    "$(ac_json_bool "$AC_SERVICE_REMOVED")" \
    "$(ac_json_bool "$AC_SERVICE_DAEMON_RELOADED")" \
    "$(ac_json_string "$AC_SERVICE_REASON")")"

# ---------------------------------------------------------------------------
# 8. Ownership record, re-render, release the lock
# ---------------------------------------------------------------------------

{
    printf 'version=%s\n' "$AC_RS_VERSION"
    printf 'plan_digest=%s\n' "$AC_PLAN_DIGEST"
    printf 'install_root=%s\n' "$AC_INSTALL_ROOT"
    printf 'bin_dir=%s\n' "$AC_BIN_DIR"
    printf 'state_dir=%s\n' "$AC_STATE_DIR"
    printf 'generation=%s\n' "$AC_GEN_DIR"
    printf 'entrypoint_sha256=%s\n' "$AC_ENTRYPOINT_SHA"
    printf 'release_set_sha256=%s\n' "$AC_RS_SHA"
    printf 'installed_at=%s\n' "$(ac_now_utc)"
} > "$AC_INSTALL_ROOT/$AC_OWNER_MARKER"

ac_render_artifacts "$AC_GEN_DIR"
ac_render_components

mkdir -p -- "$AC_STATE_DIR"
ac_preserved "$AC_STATE_DIR" "per-user state is never modified by install and is preserved for uninstall"

# Prune only after the entrypoint and ownership record are committed. A failed
# entrypoint write must keep every prior generation available for retry.
ac_gen_keep=2
ac_gen_seen=0
for ac_gen_path in $(ls -1dt "$AC_INSTALL_ROOT"/generations/* 2>/dev/null); do
    ac_gen_seen=$((ac_gen_seen + 1))
    if [ "$ac_gen_seen" -gt "$ac_gen_keep" ]; then
        rm -rf -- "$ac_gen_path"
        ac_removed "$ac_gen_path" "generation"
    fi
done

ac_release_lock

AC_EV_OUTCOME="installed"
if [ "$AC_RECOVERED" = "1" ]; then
    AC_EV_OUTCOME="recovered_and_installed"
fi
AC_EV_EXIT="$AC_EXIT_SUCCESS"
AC_EV_STATUS="ok"
AC_EV_MUTATED=1
AC_EV_MESSAGE="installed release $AC_RS_VERSION into $AC_GEN_DIR; entrypoint at $AC_BIN_DIR/axiom-cli"
ac_log "$AC_EV_MESSAGE"
ac_finish
