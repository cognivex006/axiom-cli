#!/bin/sh
# AxiomCli.Linux.Common.sh - shared helpers for the axiom-cli Linux x64 per-user
# distribution scripts.
#
# POSIX shell only. Sourced by Install-AxiomCli.sh and Uninstall-AxiomCli.sh.
# It MUST NOT use bashisms (`[[`, `function`, arrays, `$'...'`) so the install
# path works with the platform `/bin/sh` alone. The distribution contract
# (axiom-specs/contracts/axiom-cli-distribution-contract.md section 9) forbids
# requiring Bash, WSL, Docker or elevation on a platform the contract declares
# native; `/bin/sh` is not Bash and is present on every Linux host.
#
# The scripts run as a normal user, write only under the invoking user's own
# directory tree, and never invoke sudo/su/doas.

AC_PROGRAM="axiom-cli"
AC_SHELL_NAME="linux-posix-sh"
AC_SPEC_VERSION="2.0.0-draft.1"
AC_ENVELOPE_SCHEMA_VERSION=1
# Line-oriented ownership record written by the installer. It is deliberately
# not JSON: the installer reads it with `sed` so no JSON parser is required on
# the target host.
AC_OWNER_MARKER=".axiom-cli-owner"

# ---------------------------------------------------------------------------
# Logging. The envelope is the only structured payload; these go to stderr so a
# `--json` caller can parse stdout without filtering.
# ---------------------------------------------------------------------------

ac_log() {
    if [ "${AC_QUIET:-0}" = "1" ]; then
        return 0
    fi
    printf '%s: %s\n' "$AC_PROGRAM" "$1" >&2
}

ac_warn() {
    printf '%s: warning: %s\n' "$AC_PROGRAM" "$1" >&2
}

# ---------------------------------------------------------------------------
# Small utilities
# ---------------------------------------------------------------------------

ac_now_utc() {
    date -u '+%Y-%m-%dT%H:%M:%SZ'
}

ac_request_id() {
    printf '%s-%s-%s' "$AC_PROGRAM" "$$" "$(date +%s)"
}

ac_have() {
    command -v "$1" >/dev/null 2>&1
}

# Escape a value into a JSON string body (without the surrounding quotes).
ac_json_escape() {
    printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/\r/\\r/g' -e 's/\t/\\t/g' | sed -e ':a' -e 'N' -e '$!ba' -e 's/\n/\\n/g'
}

# Render a JSON string literal, or `null` when the value is empty.
ac_json_string_or_null() {
    if [ -z "${1-}" ]; then
        printf 'null'
    else
        printf '"%s"' "$(ac_json_escape "$1")"
    fi
}

# Render a JSON string literal, always quoted.
ac_json_string() {
    printf '"%s"' "$(ac_json_escape "${1-}")"
}

ac_json_bool() {
    case "${1-}" in
        1|true|yes) printf 'true' ;;
        *) printf 'false' ;;
    esac
}

ac_sha256_file() {
    sha256sum "$1" 2>/dev/null | cut -d' ' -f1
}

ac_sha256_text() {
    printf '%s' "$1" | sha256sum 2>/dev/null | cut -d' ' -f1
}

ac_trim() {
    printf '%s' "$1" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//'
}

# ---------------------------------------------------------------------------
# Install-transaction lock.
#
# The lock is a directory taken with the atomic `mkdir`, so two concurrent
# transactions cannot both win. It also carries an owner record, which means
# releasing it MUST be recursive: `rmdir` fails on a non-empty directory and
# leaves every later transaction blocked (defect found while executing L05,
# L07b and L08 of the Linux distribution suite).
# ---------------------------------------------------------------------------

ac_release_lock() {
    [ -n "${AC_INSTALL_ROOT:-}" ] || return 0
    rm -rf -- "$AC_INSTALL_ROOT/.lock" 2>/dev/null || true
    return 0
}

# Take the lock, reclaiming one left behind by a transaction that was killed
# and could not release it. Reclamation is deliberately conservative: it only
# happens when the owner record names a pid that is provably gone. A lock with
# no owner record, or one whose owner record does not name a pid, is treated as
# held (a foreign or partially written lock must never be stolen).
# Prints 0 on success and 1 when a live transaction holds it.
ac_acquire_lock() {
    if mkdir -- "$AC_INSTALL_ROOT/.lock" 2>/dev/null; then
        printf 'txn=%s\npid=%s\nstarted=%s\n' "${AC_EV_TXN:-}" "$$" "$(ac_now_utc)" \
            > "$AC_INSTALL_ROOT/.lock/owner"
        return 0
    fi
    ac_lock_pid="$(sed -n 's/^pid=//p' "$AC_INSTALL_ROOT/.lock/owner" 2>/dev/null | head -n 1)"
    if [ -z "$ac_lock_pid" ] || kill -0 "$ac_lock_pid" 2>/dev/null; then
        return 1
    fi
    ac_log "reclaiming a stale install lock left by pid $ac_lock_pid, which is no longer running"
    rm -rf -- "$AC_INSTALL_ROOT/.lock" 2>/dev/null || true
    if mkdir -- "$AC_INSTALL_ROOT/.lock" 2>/dev/null; then
        printf 'txn=%s\npid=%s\nstarted=%s\n' "${AC_EV_TXN:-}" "$$" "$(ac_now_utc)" \
            > "$AC_INSTALL_ROOT/.lock/owner"
        return 0
    fi
    return 1
}

# ---------------------------------------------------------------------------
# Envelope builder. Keys are appended in the canonical order shared with the
# Windows profile, so the emitted object always carries the same 33 keys.
# ---------------------------------------------------------------------------

AC_BODY=""

ac_env_reset() {
    AC_BODY=""
}

ac_put() {
    AC_BODY="$AC_BODY\"$1\":$2,"
}

ac_envelope_text() {
    printf '{%s}' "${AC_BODY%,}"
}

# Print the envelope to stdout and, when requested, also to a file.
ac_emit_envelope() {
    ac_line="$(ac_envelope_text)"
    printf '%s\n' "$ac_line"
    if [ -n "${AC_ENVELOPE_OUT:-}" ]; then
        mkdir -p -- "$(dirname -- "$AC_ENVELOPE_OUT")" 2>/dev/null || true
        printf '%s\n' "$ac_line" > "$AC_ENVELOPE_OUT"
    fi
}

# ---------------------------------------------------------------------------
# Host facts
# ---------------------------------------------------------------------------

ac_os_release_field() {
    [ -r /etc/os-release ] || return 1
    sed -n "s/^$1=//p" /etc/os-release | head -n 1 | sed -e 's/^"//' -e 's/"$//'
}

ac_host_arch() {
    uname -m
}

ac_host_kernel() {
    uname -srmo 2>/dev/null || uname -a
}

ac_host_libc() {
    if ac_have getconf; then
        ac_getconf_line="$(getconf GNU_LIBC_VERSION 2>/dev/null || true)"
        if [ -n "$ac_getconf_line" ]; then
            printf '%s' "$ac_getconf_line"
            return 0
        fi
    fi
    if ac_have ldd; then
        ldd --version 2>/dev/null | head -n 1
        return 0
    fi
    printf 'unknown'
}

ac_host_is_wsl() {
    if [ -n "${WSL_DISTRO_NAME:-}" ]; then
        printf '1'
        return 0
    fi
    if [ -r /proc/version ] && grep -qi 'microsoft' /proc/version 2>/dev/null; then
        printf '1'
        return 0
    fi
    printf '0'
}

ac_session_uid() {
    id -u 2>/dev/null || printf '0'
}

ac_session_elevated() {
    if [ "$(ac_session_uid)" = "0" ]; then
        printf '1'
    else
        printf '0'
    fi
}

# Is a `systemd --user` manager actually reachable? The unit-registration step
# uses this and degrades explicitly when it is false.
ac_systemd_user_available() {
    ac_have systemctl || return 1
    [ -d /run/systemd/system ] || return 1
    systemctl --user show-environment >/dev/null 2>&1 || return 1
    return 0
}

# A reason string that states exactly which precondition failed.
ac_systemd_user_reason() {
    if ! ac_have systemctl; then
        printf 'systemctl is not installed, so no user service manager is reachable'
        return 0
    fi
    if [ ! -d /run/systemd/system ]; then
        printf 'systemd is not the running init in this environment (/run/systemd/system is absent), so no user service manager is reachable'
        return 0
    fi
    if ! systemctl --user show-environment >/dev/null 2>&1; then
        printf 'systemd is running but `systemctl --user` does not answer for uid %s, so no user service manager is reachable' "$(ac_session_uid)"
        return 0
    fi
    printf 'a user service manager answered `systemctl --user show-environment`'
}

# Render the `host` block as JSON.
ac_host_json() {
    ac_h_kernel="$(ac_host_kernel)"
    ac_h_distro_id="$(ac_os_release_field ID || true)"
    [ -n "$ac_h_distro_id" ] || ac_h_distro_id="unknown"
    ac_h_distro_version="$(ac_os_release_field VERSION_ID || true)"
    [ -n "$ac_h_distro_version" ] || ac_h_distro_version="unknown"
    ac_h_libc="$(ac_host_libc)"
    ac_h_wsl="$(ac_host_is_wsl)"
    if [ "$ac_h_wsl" = "1" ]; then
        ac_h_wsl_name="$(ac_json_string_or_null "${WSL_DISTRO_NAME:-}")"
    else
        ac_h_wsl_name="null"
    fi
    printf '{"os":"linux","arch":%s,"kernel":%s,"distro_id":%s,"distro_version":%s,"libc":%s,"wsl":%s,"wsl_distro_name":%s,"session_uid":%s,"session_elevated":%s,"systemd_user_available":%s}' \
        "$(ac_json_string "$(ac_host_arch)")" \
        "$(ac_json_string "$ac_h_kernel")" \
        "$(ac_json_string "$ac_h_distro_id")" \
        "$(ac_json_string "$ac_h_distro_version")" \
        "$(ac_json_string "$ac_h_libc")" \
        "$(ac_json_bool "$ac_h_wsl")" \
        "$ac_h_wsl_name" \
        "$(ac_session_uid)" \
        "$(ac_json_bool "$(ac_session_elevated)")" \
        "$(ac_json_bool "$(ac_systemd_user_available && printf 1 || printf 0)")"
}
