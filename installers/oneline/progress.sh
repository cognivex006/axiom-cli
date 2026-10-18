# One bootstrap display process. Its stdin is always /dev/null, never piped script input.
ux_pid=''
ux_worker=''
ux_meter=''
ux_state=''
ux_mode=plain
ux_start() {
    case "${AXIOM_PROGRESS:-auto}" in off) ux_mode=off; return 0 ;; plain) return 0 ;; esac
    [ -t 1 ] && [ -t 2 ] && [ -z "${CI+x}" ] && [ "${TERM:-dumb}" != dumb ] || return 0
    ux_size=$(stty size 2>/dev/null </dev/tty) || return 0
    set -- $ux_size
    [ "$#" = 2 ] && [ "$1" -ge 3 ] && [ "$2" -ge 32 ] || return 0
    # Keep display IPC local even when the archive's TMPDIR is a slow mount.
    ux_state=$(mktemp -d /tmp/axiom-ux.XXXXXX 2>/dev/null) || return 0
    ux_mode=live
    ux_observe </dev/null &
    ux_meter=$!
    ux_loop </dev/null &
    ux_pid=$!
}
ux_observe() {
    ux_observer_child=''
    ux_observer_stop() {
        if [ -n "$ux_observer_child" ]; then kill -TERM "$ux_observer_child" 2>/dev/null || :; fi
        exit 0
    }
    trap 'ux_observer_stop' TERM INT HUP
    while :; do
        ux_path=''
        if [ -f "$ux_state/progress.state" ]; then
            { IFS= read -r ux_top; IFS= read -r ux_text; IFS= read -r ux_path; } <"$ux_state/progress.state" || :
        fi
        if [ -n "$ux_path" ]; then
            # File/header observation can stall; it never owns the display timer.
            wc -c 2>/dev/null <"$ux_path" >"$ux_state/progress.bytes" &
            ux_observer_child=$!
            ux_bytes=''
            if wait "$ux_observer_child"; then IFS= read -r ux_bytes <"$ux_state/progress.bytes" || :; fi
            ux_observer_child=''
            ux_headers="$tmp/headers"
            [ "$fetcher" = curl ] || ux_headers="$tmp/download.error"
            awk 'tolower($1) ~ /^http\// {n=0} tolower($1)=="content-length:" {gsub("\r", "", $2); if($2 ~ /^[0-9]+$/) n=$2} END {printf "%.0f\n", (n>0 && n<=9007199254740991?n:0)}' "$ux_headers" >"$ux_state/progress.total" 2>/dev/null &
            ux_observer_child=$!
            ux_total=0
            if wait "$ux_observer_child"; then IFS= read -r ux_total <"$ux_state/progress.total" || :; fi
            ux_observer_child=''
            # Whitespace from wc is harmless to awk; failed reads stay unknown.
            awk -v b="$ux_bytes" -v t="$ux_total" 'BEGIN {p=-1; if(b ~ /^[ ]*[0-9]+[ ]*$/ && t>0) {p=int(b*100/t); if(p>99)p=99} print p}' >"$ux_state/progress.percent"
            { printf '%s\n' "$ux_path"; cat "$ux_state/progress.percent"; } >"$ux_state/progress.measure.next"
            mv "$ux_state/progress.measure.next" "$ux_state/progress.measure"
        fi
        sleep 0.125 &
        ux_observer_child=$!
        wait "$ux_observer_child" || :
        ux_observer_child=''
    done
}
ux_stage() {
    ux_label=$1
    ux_detail=$2
    [ "$ux_mode" != off ] || return 0
    if [ "$ux_mode" = live ]; then
        # Only printable ASCII targets enter the bootstrap display; no terminal controls.
        ux_safe_label=$(printf '%s' "$1" | LC_ALL=C tr -cd '\40-\176')
        ux_safe_detail=$(printf '%s' "$2" | LC_ALL=C tr -cd '\40-\176')
        ux_safe_path=$(printf '%s' "${3:-}" | LC_ALL=C tr -d '\000-\037\177')
        [ "$ux_safe_path" = "${3:-}" ] || ux_safe_path=''
        printf '%s\n%s\n%s\n' "$ux_safe_label" "$ux_safe_detail" "$ux_safe_path" >"$ux_state/progress.next"
        mv "$ux_state/progress.next" "$ux_state/progress.state"
    else
        printf 'axiom %s: %s\n' "$AXIOM_ACTION" "$2" >&2
    fi
}
ux_loop() {
    ux_reserved=0
    ux_frame=0
    ux_clear() {
        if [ "$ux_reserved" = 1 ]; then
            ux_anchor=''
            ux_size=$(stty size 2>/dev/null </dev/tty) || ux_size=''
            set -- $ux_size
            if [ "$#" = 2 ] && { [ "${ux_last_width:-}" != "$(($2 - 1))" ] || [ "${ux_last_height:-}" != "$1" ]; }; then ux_anchor='\r\0337'; fi
            printf '%b\0338\r\033[J\033[?25h' "$ux_anchor" >&2
            ux_reserved=0
        fi
    }
    trap 'ux_clear; exit 0' TERM INT HUP
    while :; do
        if [ -f "$ux_state/progress.state" ]; then
            ux_size=$(stty size 2>/dev/null </dev/tty) || break
            set -- $ux_size
            [ "$#" = 2 ] || break
            ux_width=$(($2 - 1))
            # Read one atomic snapshot, without mixing phases across sed forks.
            ux_top=''; ux_text=''; ux_path=''
            { IFS= read -r ux_top; IFS= read -r ux_text; IFS= read -r ux_path; } <"$ux_state/progress.state" || :
            if [ "$ux_width" -ge 31 ] && [ "$1" -ge 3 ]; then
                ux_anchor=''
                if [ "$ux_reserved" = 1 ] && { [ "${ux_last_width:-}" != "$ux_width" ] || [ "${ux_last_height:-}" != "$1" ]; }; then ux_anchor='\r\0337'; fi
                ux_last_width=$ux_width
                ux_last_height=$1
                ux_percent=-1
                if [ -n "$ux_path" ] && [ -f "$ux_state/progress.measure" ]; then
                    { IFS= read -r ux_observed; IFS= read -r ux_sample; } <"$ux_state/progress.measure" || :
                    if [ "$ux_observed" = "$ux_path" ]; then
                        case "$ux_sample" in -1|[0-9]|[0-9][0-9]) ux_percent=$ux_sample ;; esac
                    fi
                fi
                ux_bar=''
                ux_count=$((ux_width / 3)); [ "$ux_count" -le 40 ] || ux_count=40
                ux_i=0
                while [ "$ux_i" -lt "$ux_count" ]; do
                    if { [ "$ux_percent" -ge 0 ] && [ "$ux_i" -lt "$((ux_percent * ux_count / 100))" ]; } || { [ "$ux_percent" -lt 0 ] && [ "$ux_i" -eq "$((ux_frame % ux_count))" ]; }; then ux_bar="${ux_bar}="; else ux_bar="${ux_bar}-"; fi
                    ux_i=$((ux_i + 1))
                done
                ux_status=indeterminate
                [ "$ux_percent" -lt 0 ] || ux_status="${ux_percent}%"
                ux_labelwidth=$((ux_width - ux_count - ${#ux_status} - 5))
                [ "$ux_labelwidth" -ge 0 ] || ux_labelwidth=0
                ux_top=$(printf '%.*s' "$ux_labelwidth" "$ux_top")
                case "$((ux_frame % 4))" in 0) ux_spin='|' ;; 1) ux_spin='/' ;; 2) ux_spin='-' ;; 3) ux_spin='\' ;; esac
                if [ "$ux_reserved" = 0 ]; then printf '\033[?25l\r\n\r\n\033[2A\0337' >&2; ux_reserved=1; fi
                printf '%b\0338\r\033[J\033[2K%.*s\r\n\033[2K%.*s\0338' "$ux_anchor" "$ux_width" "$ux_top  [$ux_bar] $ux_status" "$ux_width" "$ux_spin axiom $AXIOM_ACTION: $ux_text" >&2
                ux_frame=$(((ux_frame + 1) % 120000))
            else
                ux_clear
                if [ "${ux_fallback:-}" != "$ux_text" ]; then printf 'axiom %s: %s\n' "$AXIOM_ACTION" "$ux_text" >&2; ux_fallback=$ux_text; fi
            fi
        fi
        sleep 0.125
    done
    ux_clear
}
ux_dispose() {
    if [ -n "$ux_pid" ]; then kill -TERM "$ux_pid" 2>/dev/null || :; wait "$ux_pid" 2>/dev/null || :; ux_pid=''; fi
    if [ -n "$ux_meter" ]; then kill -TERM "$ux_meter" 2>/dev/null || :; wait "$ux_meter" 2>/dev/null || :; ux_meter=''; fi
    case "$ux_state" in /tmp/axiom-ux.*) rm -rf "$ux_state"; ux_state='' ;; esac
}
ux_cleanup() {
    ux_dispose
    if [ -n "$ux_worker" ]; then kill -TERM "$ux_worker" 2>/dev/null || :; wait "$ux_worker" 2>/dev/null || :; ux_worker=''; fi
    rm -rf "$tmp"
}
ux_fetch() {
    # Preserve original transfer diagnostics until the renderer is suspended.
    if [ "$fetcher" = curl ]; then
        curl -fLsS --retry 2 --dump-header "$tmp/headers" -o "$2" "$1" 2>"$tmp/download.error" </dev/null &
    else
        wget -q --server-response -O "$2" "$1" 2>"$tmp/download.error" </dev/null &
    fi
    ux_worker=$!
    ux_result=0
    wait "$ux_worker" || ux_result=$?
    ux_worker=''
    if [ "$ux_result" -ne 0 ]; then
        ux_dispose
        # Wget headers are remote text, never terminal instructions. Keep UTF-8
        # diagnostics and newlines/tabs, remove C0/C1 and bidi formatting controls.
        LC_ALL=C tr -cd '\11\12\40-\176\200-\377' <"$tmp/download.error" |
            LC_ALL=C awk '{gsub(/\302[\200-\237]|\342\200[\213\216\217\252-\256]|\342\201[\246-\251]/, ""); print}' >&2
    fi
    return "$ux_result"
}
