#!/bin/sh
# Per-user Intel Mac bootstrap for the two public CLI entrypoints.
# Core lifecycle, service control and data removal remain owned by `axiom`.
set -eu

usage() {
    printf '%s\n' 'Usage: Manage-Entrypoints.sh install --release-set DIR (--dry-run|--apply) [--approve-digest SHA256]'
    printf '%s\n' '       Manage-Entrypoints.sh uninstall (--dry-run|--apply) [--approve-digest SHA256]'
}
die() {
    code=$1 reason=$2
    printf 'entrypoints: %s\n' "$reason" >&2
    printf '{"status":"refused","reason":"%s","exit_code":%s}\n' "$reason" "$code"
    exit "$code"
}
sha() { shasum -a 256 "$1" | awk '{print $1}'; }
field() { plutil -extract "$2" raw -o - "$1" 2>/dev/null; }
line() { sed -n "${2}p" "$1"; }
safe_path() {
    case "$1" in /*) ;; *) die 2 relative_path ;; esac
    if printf '%s' "$1" | LC_ALL=C grep -q '[[:cntrl:]]'; then die 2 control_character_in_path; fi
}

[ "$(uname -s):$(uname -m)" = Darwin:x86_64 ] || die 9 macos_x64_required
[ "$(id -u)" -ne 0 ] || die 5 root_user_refused
[ -n "${HOME:-}" ] || die 4 home_missing
safe_path "$HOME"
case "$HOME" in */|*'//'*) die 2 noncanonical_home ;; esac
ROOT=${AXIOM_CLI_INSTALL_ROOT:-"$HOME/.local/share/axiom"}
safe_path "$ROOT"
case "$ROOT" in "$HOME"/*) ;; *) die 5 install_root_outside_user_home ;; esac
case "$ROOT" in */../*|*/..|*/./*|*/.|*'//'*) die 2 noncanonical_install_root ;; esac
BIN="$HOME/.local/bin"
cursor=$ROOT
while [ "$cursor" != "$HOME" ]; do
    [ ! -L "$cursor" ] || die 6 symlink_at_managed_path
    cursor=$(dirname "$cursor")
done
[ ! -L "$HOME/.local" ] && [ ! -L "$BIN" ] || die 6 symlink_at_managed_path
case "${SHELL:-}" in
    */bash) PROFILE="$HOME/.bash_profile" ;;
    */zsh) PROFILE="$HOME/.zprofile" ;;
    *) die 9 unsupported_user_shell ;;
esac
BASE="$ROOT/entrypoints"
STATE="$BASE/current.tsv"
CLI_TARGET="$BIN/axiom-cli"
ENGINE_TARGET="$BIN/axiom"
VERB=${1:-}
[ -n "$VERB" ] || { usage; exit 2; }
shift
SET="" MODE="" APPROVAL=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --release-set|--approve-digest)
            [ "$#" -ge 2 ] || die 2 missing_option_value
            case "$1" in --release-set) SET=$2 ;; *) APPROVAL=$2 ;; esac
            shift 2 ;;
        --dry-run|--apply)
            [ -z "$MODE" ] || die 2 duplicate_mode
            MODE=$1; shift ;;
        --help|-h) usage; exit 0 ;;
        *) die 2 unknown_option ;;
    esac
done
case "$VERB:$MODE" in install:--dry-run|install:--apply|uninstall:--dry-run|uninstall:--apply) ;; *) die 2 invalid_mode ;; esac

# A generated block has fixed bytes and uses the current user's home at shell
# startup. The block does not embed a path supplied by the release set.
BLOCK='# axiom-cli PATH begin
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) PATH="$HOME/.local/bin:$PATH";; esac
export PATH
# axiom-cli PATH end'
profile_state() {
    [ -f "$PROFILE" ] || { printf absent; return; }
    [ ! -L "$PROFILE" ] || { printf edited; return; }
    starts=$(grep -c '^# axiom-cli PATH begin$' "$PROFILE" || true)
    ends=$(grep -c '^# axiom-cli PATH end$' "$PROFILE" || true)
    [ "$starts" -eq 0 ] && [ "$ends" -eq 0 ] && { printf absent; return; }
    [ "$starts" -eq 1 ] && [ "$ends" -eq 1 ] || { printf edited; return; }
    actual=$(sed -n '/^# axiom-cli PATH begin$/,/^# axiom-cli PATH end$/p' "$PROFILE")
    [ "$actual" = "$BLOCK" ] && printf owned || printf edited
}
record() {
    [ -f "$STATE" ] || die 3 ownership_record_missing
    [ "$(line "$STATE" 1)" = axiom-cli-entrypoints-v1 ] || die 9 ownership_record_incompatible
    REC_SET=$(line "$STATE" 2)
    REC_CLI_VERSION=$(line "$STATE" 3)
    REC_ENGINE_VERSION=$(line "$STATE" 4)
    REC_CORE_REVISION=$(line "$STATE" 5)
    REC_CLI=$(line "$STATE" 6)
    REC_ENGINE=$(line "$STATE" 7)
    REC_PROFILE=$(line "$STATE" 8)
    [ "$REC_PROFILE" = "$PROFILE" ] || die 6 shell_profile_changed
    for version in "$REC_CLI_VERSION" "$REC_ENGINE_VERSION"; do
        [ -n "$version" ] || die 9 ownership_record_incompatible
        case "$version" in *[!0-9A-Za-z.+-]*) die 9 ownership_record_incompatible ;; esac
    done
    [ "${#REC_CORE_REVISION}" -eq 40 ] || die 9 ownership_record_incompatible
    case "$REC_CORE_REVISION" in *[!0-9a-f]*) die 9 ownership_record_incompatible ;; esac
    for digest in "$REC_SET" "$REC_CLI" "$REC_ENGINE"; do
        case "$digest" in *[!0-9a-f]*) die 9 ownership_record_incompatible ;; esac
        [ "${#digest}" -eq 64 ] || die 9 ownership_record_incompatible
    done
}
owned_files() {
    [ -f "$CLI_TARGET" ] && [ ! -L "$CLI_TARGET" ] && [ "$(sha "$CLI_TARGET")" = "$REC_CLI" ] || die 6 cli_entrypoint_changed
    [ -f "$ENGINE_TARGET" ] && [ ! -L "$ENGINE_TARGET" ] && [ "$(sha "$ENGINE_TARGET")" = "$REC_ENGINE" ] || die 6 engine_entrypoint_changed
    [ "$(profile_state)" = owned ] || die 6 shell_profile_changed
    VERSION_DIR="$BASE/versions/$REC_SET"
    [ -f "$VERSION_DIR/axiom-cli" ] && [ ! -L "$VERSION_DIR/axiom-cli" ] && [ "$(sha "$VERSION_DIR/axiom-cli")" = "$REC_CLI" ] || die 6 owned_cli_generation_changed
    [ -f "$VERSION_DIR/axiom" ] && [ ! -L "$VERSION_DIR/axiom" ] && [ "$(sha "$VERSION_DIR/axiom")" = "$REC_ENGINE" ] || die 6 owned_engine_generation_changed
}

if [ "$VERB" = install ]; then
    [ -n "$SET" ] || die 2 release_set_required
    if [ -d "$SET" ]; then SET="$SET/release-set.json"; fi
    [ -f "$SET" ] && [ ! -L "$SET" ] || die 3 release_set_missing
    [ "$(field "$SET" release_set_version)" = 1 ] || die 9 release_set_version_incompatible
    [ "$(field "$SET" platform)" = macos-x64 ] || die 9 release_set_platform_incompatible
    [ "$(field "$SET" arch)" = x86_64 ] || die 9 release_set_arch_incompatible
    [ "$(field "$SET" signing)" = unsigned ] || die 9 signing_status_unverified
    [ "$(field "$SET" notarization)" = not_notarized ] || die 9 notarization_status_unverified
    CLI_VERSION=$(field "$SET" artifacts.0.version) || die 2 release_set_artifacts_invalid
    ENGINE_VERSION=$(field "$SET" artifacts.1.version) || die 2 release_set_artifacts_invalid
    CORE_REVISION=$(field "$SET" artifacts.1.revision) || die 2 release_set_artifacts_invalid
    [ -n "$CLI_VERSION" ] && [ -n "$ENGINE_VERSION" ] || die 2 release_set_artifacts_invalid
    for version in "$CLI_VERSION" "$ENGINE_VERSION"; do
        case "$version" in *[!0-9A-Za-z.+-]*) die 2 release_version_invalid ;; esac
    done
    [ "${#CORE_REVISION}" -eq 40 ] || die 2 release_core_revision_invalid
    case "$CORE_REVISION" in *[!0-9a-f]*) die 2 release_core_revision_invalid ;; esac
    [ "$(field "$SET" artifacts.0.name)" = axiom-cli ] || die 2 release_set_artifacts_invalid
    [ "$(field "$SET" artifacts.1.name)" = axiom ] || die 2 release_set_artifacts_invalid
    [ "$(field "$SET" artifacts.2.name)" = axiom-graphd ] || die 2 release_set_artifacts_invalid
    [ "$(field "$SET" artifacts.2.version)" = "$ENGINE_VERSION" ] || die 9 core_versions_disagree
    [ "$(field "$SET" artifacts.2.revision)" = "$CORE_REVISION" ] || die 9 core_revisions_disagree
    SET_DIR=$(CDPATH= cd -- "$(dirname -- "$SET")" && pwd)
    CLI_SOURCE="$SET_DIR/axiom-cli"
    ENGINE_SOURCE="$SET_DIR/axiom"
    for pair in '0 axiom-cli' '1 axiom' '2 axiom-graphd'; do
        index=${pair%% *}; name=${pair#* }
        path="$SET_DIR/$name"
        [ "$(field "$SET" "artifacts.$index.component")" = "$name" ] || die 2 release_set_artifacts_invalid
        [ "$(field "$SET" "artifacts.$index.archive")" = "$name" ] || die 2 release_set_artifacts_invalid
        [ -f "$path" ] && [ ! -L "$path" ] && [ -x "$path" ] || die 3 release_artifact_missing
        magic=$(od -An -tx1 -N4 -- "$path" | tr -d ' \n')
        cpu=$(od -An -tx1 -j4 -N4 -- "$path" | tr -d ' \n')
        [ "$magic:$cpu" = cffaedfe:07000001 ] || die 9 release_artifact_arch_incompatible
        expected=$(field "$SET" "artifacts.$index.sha256") || die 2 release_set_artifacts_invalid
        bytes=$(field "$SET" "artifacts.$index.size_bytes") || die 2 release_set_artifacts_invalid
        [ "${#expected}" -eq 64 ] && [ "$(sha "$path")" = "$expected" ] || die 9 release_artifact_digest_mismatch
        [ "$(wc -c < "$path" | tr -d ' ')" = "$bytes" ] || die 9 release_artifact_size_mismatch
    done
    SET_SHA=$(sha "$SET")
    CLI_SHA=$(field "$SET" artifacts.0.sha256)
    ENGINE_SHA=$(field "$SET" artifacts.1.sha256)
    PLAN=$(printf '%s\n' install "$SET_SHA" "$ROOT" "$BIN" "$PROFILE" | shasum -a 256 | awk '{print $1}')
    if [ -f "$STATE" ]; then
        record
        [ "$REC_SET" = "$SET_SHA" ] || die 6 update_requires_ecosystem_transaction
        owned_files
        ALREADY=1
    else
        [ ! -e "$CLI_TARGET" ] && [ ! -L "$CLI_TARGET" ] || die 6 unowned_cli_entrypoint
        [ ! -e "$ENGINE_TARGET" ] && [ ! -L "$ENGINE_TARGET" ] || die 6 unowned_engine_entrypoint
        [ "$(profile_state)" = absent ] || die 6 unowned_or_edited_profile_block
        ALREADY=0
    fi
else
    [ -z "$SET" ] || die 2 uninstall_has_no_release_set
    record
    owned_files
    SET_SHA=$REC_SET
    CLI_VERSION=$REC_CLI_VERSION
    ENGINE_VERSION=$REC_ENGINE_VERSION
    CORE_REVISION=$REC_CORE_REVISION
    CLI_SHA=$REC_CLI
    ENGINE_SHA=$REC_ENGINE
    PLAN=$(printf '%s\n' uninstall "$(sha "$STATE")" "$ROOT" "$BIN" "$PROFILE" | shasum -a 256 | awk '{print $1}')
    ALREADY=0
fi

emit() {
    printf '{"status":"%s","verb":"%s","plan_digest":"%s","manifest_sha256":"%s","already_installed":%s,"artifacts":[{"component":"axiom-cli","version":"%s","sha256":"%s"},{"component":"axiom","version":"%s","revision":"%s","sha256":"%s"}]}\n' \
        "$1" "$VERB" "$PLAN" "$SET_SHA" "$ALREADY" "$CLI_VERSION" "$CLI_SHA" "$ENGINE_VERSION" "$CORE_REVISION" "$ENGINE_SHA"
}
if [ "$MODE" = --dry-run ]; then
    emit planned
    exit 0
fi
[ "$APPROVAL" = "$PLAN" ] || die 6 approval_stale
if [ "$VERB" = install ] && [ "$ALREADY" -eq 1 ]; then
    emit already_installed
    exit 0
fi

for path in "$ROOT" "$BASE" "$BIN" "$BASE/versions" "$PROFILE" "$STATE"; do
    [ ! -L "$path" ] || die 6 symlink_at_managed_path
done
mkdir -p "$BASE" || die 8 install_root_unwritable
mkdir "$BASE/.lock" 2>/dev/null || die 10 transaction_lock_unavailable
TXN=$(mktemp -d "$BASE/.txn.XXXXXX") || { rmdir "$BASE/.lock"; die 8 transaction_stage_unavailable; }
COMMITTED=0
MUTATED=0
PROFILE_EXISTED=0
cleanup() {
    if [ "$COMMITTED" -eq 0 ] && [ "$MUTATED" -eq 1 ]; then
        if [ "$VERB" = install ]; then
            [ ! -f "$CLI_TARGET" ] || { [ "$(sha "$CLI_TARGET")" != "$CLI_SHA" ] || rm -f "$CLI_TARGET"; }
            [ ! -f "$ENGINE_TARGET" ] || { [ "$(sha "$ENGINE_TARGET")" != "$ENGINE_SHA" ] || rm -f "$ENGINE_TARGET"; }
            if [ "$PROFILE_EXISTED" -eq 1 ]; then cp -p "$TXN/profile.before" "$PROFILE"; else rm -f "$PROFILE"; fi
            rm -f "$BASE/versions/$SET_SHA/axiom-cli" "$BASE/versions/$SET_SHA/axiom"
            rmdir "$BASE/versions/$SET_SHA" 2>/dev/null || true
        else
            [ ! -f "$TXN/axiom-cli.before" ] || cp -p "$TXN/axiom-cli.before" "$CLI_TARGET"
            [ ! -f "$TXN/axiom.before" ] || cp -p "$TXN/axiom.before" "$ENGINE_TARGET"
            [ ! -f "$TXN/profile.before" ] || cp -p "$TXN/profile.before" "$PROFILE"
            [ ! -f "$TXN/state.before" ] || cp -p "$TXN/state.before" "$STATE"
            mkdir -p "$BASE/versions/$SET_SHA" 2>/dev/null || true
            [ ! -f "$TXN/axiom-cli.before" ] || cp -p "$TXN/axiom-cli.before" "$BASE/versions/$SET_SHA/axiom-cli"
            [ ! -f "$TXN/axiom.before" ] || cp -p "$TXN/axiom.before" "$BASE/versions/$SET_SHA/axiom"
        fi
    fi
    rm -f "$BIN/.axiom-cli.$$" "$BIN/.axiom.$$" 2>/dev/null || true
    rm -f "$TXN"/* 2>/dev/null || true
    rmdir "$TXN" 2>/dev/null || true
    rmdir "$BASE/.lock" 2>/dev/null || true
}
trap cleanup EXIT HUP INT TERM

if [ "$VERB" = install ]; then
    [ ! -e "$STATE" ] && [ ! -e "$BASE/versions/$SET_SHA" ] || die 6 ownership_changed_during_apply
    [ ! -e "$CLI_TARGET" ] && [ ! -L "$CLI_TARGET" ] && [ ! -e "$ENGINE_TARGET" ] && [ ! -L "$ENGINE_TARGET" ] || die 6 entrypoint_changed_during_apply
    [ "$(profile_state)" = absent ] || die 6 profile_changed_during_apply
    mkdir -p "$BIN" "$BASE/versions" || die 8 user_bin_unwritable
    [ ! -L "$BIN" ] && [ ! -L "$BASE/versions" ] || die 6 symlink_at_managed_path
    cp "$CLI_SOURCE" "$TXN/axiom-cli" && cp "$ENGINE_SOURCE" "$TXN/axiom" || die 8 stage_copy_failed
    chmod 755 "$TXN/axiom-cli" "$TXN/axiom" || die 8 stage_mode_failed
    [ "$(sha "$TXN/axiom-cli")" = "$CLI_SHA" ] && [ "$(sha "$TXN/axiom")" = "$ENGINE_SHA" ] || die 9 stage_digest_mismatch
    if [ -f "$PROFILE" ]; then cp -p "$PROFILE" "$TXN/profile.before" || die 8 profile_backup_failed; PROFILE_EXISTED=1; fi
    mkdir "$BASE/versions/$SET_SHA" || die 8 version_directory_unwritable
    MUTATED=1
    cp "$TXN/axiom-cli" "$BASE/versions/$SET_SHA/axiom-cli" || die 8 version_copy_failed
    cp "$TXN/axiom" "$BASE/versions/$SET_SHA/axiom" || die 8 version_copy_failed
    cp "$TXN/axiom-cli" "$BIN/.axiom-cli.$$" && cp "$TXN/axiom" "$BIN/.axiom.$$" || die 8 bin_stage_failed
    chmod 755 "$BIN/.axiom-cli.$$" "$BIN/.axiom.$$" || die 8 bin_mode_failed
    mv "$BIN/.axiom-cli.$$" "$CLI_TARGET" && mv "$BIN/.axiom.$$" "$ENGINE_TARGET" || die 8 bin_activation_failed
    if [ -f "$PROFILE" ]; then cp -p "$PROFILE" "$TXN/profile.new"; else : > "$TXN/profile.new"; chmod 600 "$TXN/profile.new"; fi
    printf '\n%s\n' "$BLOCK" >> "$TXN/profile.new"
    mv "$TXN/profile.new" "$PROFILE" || die 8 profile_write_failed
    [ "${AXIOM_CLI_TEST_FAIL_AFTER_PROFILE:-}" != 1 ] || die 8 injected_failure
    printf 'axiom-cli-entrypoints-v1\n%s\n%s\n%s\n%s\n%s\n%s\n%s\n' "$SET_SHA" "$CLI_VERSION" "$ENGINE_VERSION" "$CORE_REVISION" "$CLI_SHA" "$ENGINE_SHA" "$PROFILE" > "$TXN/state.new"
    mv "$TXN/state.new" "$STATE" || die 8 state_write_failed
else
    cp -p "$CLI_TARGET" "$TXN/axiom-cli.before" && cp -p "$ENGINE_TARGET" "$TXN/axiom.before" && cp -p "$PROFILE" "$TXN/profile.before" && cp -p "$STATE" "$TXN/state.before" || die 8 uninstall_backup_failed
    MUTATED=1
    sed '/^# axiom-cli PATH begin$/,/^# axiom-cli PATH end$/d' "$PROFILE" > "$TXN/profile.new" || die 8 profile_edit_failed
    chmod "$(stat -f %Lp "$PROFILE")" "$TXN/profile.new" || die 8 profile_mode_failed
    mv "$TXN/profile.new" "$PROFILE" || die 8 profile_write_failed
    rm "$CLI_TARGET" "$ENGINE_TARGET" || die 8 entrypoint_removal_failed
    [ "${AXIOM_CLI_TEST_FAIL_AFTER_REMOVAL:-}" != 1 ] || die 8 injected_failure
    rm "$BASE/versions/$SET_SHA/axiom-cli" "$BASE/versions/$SET_SHA/axiom" || die 8 version_removal_failed
    rmdir "$BASE/versions/$SET_SHA" || die 8 version_removal_failed
    rm "$STATE" || die 8 state_removal_failed
fi
COMMITTED=1
if [ "$VERB" = install ]; then emit installed; else emit uninstalled; fi
