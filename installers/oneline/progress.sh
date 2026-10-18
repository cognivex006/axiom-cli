# One bootstrap display process. Its stdin is always /dev/null, never piped script input.
ux_pid=''
ux_worker=''
ux_meter=''
ux_mode=plain
ux_start() {
    case "${AXIOM_PROGRESS:-auto}" in off) ux_mode=off; return ;; plain) return ;; esac
    [ -t 1 ] && [ -t 2 ] && [ -z "${CI+x}" ] && [ "${TERM:-dumb}" != dumb ] || return
    ux_size=$(stty size 2>/dev/null </dev/tty) || return
    set -- $ux_size
    [ "$#" = 2 ] && [ "$1" -ge 3 ] && [ "$2" -ge 32 ] || return
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
        if [ -f "$tmp/progress.state" ]; then
            { IFS= read -r ux_top; IFS= read -r ux_text; IFS= read -r ux_path; } <"$tmp/progress.state" || :
        fi
        if [ -n "$ux_path" ]; then
            # File/header observation can stall; it never owns the display timer.
            wc -c <"$ux_path" >"$tmp/progress.bytes" 2>/dev/null &
            ux_observer_child=$!
            ux_bytes=''
            if wait "$ux_observer_child"; then IFS= read -r ux_bytes <"$tmp/progress.bytes" || :; fi
            ux_observer_child=''
            ux_headers="$tmp/headers"
            [ "$fetcher" = curl ] || ux_headers="$tmp/download.error"
            awk 'tolower($1) ~ /^http\// {n=0} tolower($1)=="content-length:" {gsub("\r", "", $2); if($2 ~ /^[0-9]+$/) n=$2} END {printf "%.0f\n", (n>0 && n<=9007199254740991?n:0)}' "$ux_headers" >"$tmp/progress.total" 2>/dev/null &
            ux_observer_child=$!
            ux_total=0
            if wait "$ux_observer_child"; then IFS= read -r ux_total <"$tmp/progress.total" || :; fi
            ux_observer_child=''
            # Whitespace from wc is harmless to awk; failed reads stay unknown.
            awk -v b="$ux_bytes" -v t="$ux_total" 'BEGIN {p=-1; if(b ~ /^[ ]*[0-9]+[ ]*$/ && t>0) {p=int(b*100/t); if(p>99)p=99} print p}' >"$tmp/progress.percent"
            { printf '%s\n' "$ux_path"; cat "$tmp/progress.percent"; } >"$tmp/progress.measure.next"
            mv "$tmp/progress.measure.next" "$tmp/progress.measure"
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
        printf '%s\n%s\n%s\n' "$ux_safe_label" "$ux_safe_detail" "$ux_safe_path" >"$tmp/progress.next"
        mv "$tmp/progress.next" "$tmp/progress.state"
    else
        printf 'axiom %s: %s\n' "$AXIOM_ACTION" "$2" >&2
    fi
}
ux_loop() {
    ux_reserved=0
    ux_frame=0
    ux_clear() {
        if [ "$ux_reserved" = 1 ]; then printf '\0338\r\033[2K\r\n\033[2K\0338\033[?25h' >&2; ux_reserved=0; fi
    }
    trap 'ux_clear; exit 0' TERM INT HUP
    while :; do
        if [ -f "$tmp/progress.state" ]; then
            ux_size=$(stty size 2>/dev/null </dev/tty) || break
            set -- $ux_size
            [ "$#" = 2 ] || break
            ux_width=$(($2 - 1))
            if [ "$ux_width" -ge 31 ] && [ "$1" -ge 3 ]; then
                ux_top=$(sed -n '1p' "$tmp/progress.state")
                ux_text=$(sed -n '2p' "$tmp/progress.state")
                ux_path=$(sed -n '3p' "$tmp/progress.state")
                ux_percent=-1
                if [ -n "$ux_path" ] && [ -f "$tmp/progress.measure" ]; then
                    { IFS= read -r ux_observed; IFS= read -r ux_sample; } <"$tmp/progress.measure" || :
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
                printf '\0338\r\033[2K%.*s\r\n\033[2K%.*s\0338' "$ux_width" "$ux_top  [$ux_bar] $ux_status" "$ux_width" "$ux_spin axiom $AXIOM_ACTION: $ux_text" >&2
                ux_frame=$(((ux_frame + 1) % 120000))
            else
                ux_clear
                ux_text=$(sed -n '2p' "$tmp/progress.state")
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
        LC_ALL=C tr -cd '\11\12\40-\377' <"$tmp/download.error" |
            LC_ALL=C awk '{gsub(/\302[\200-\237]|\342\200[\213\216\217\252-\256]|\342\201[\246-\251]/, ""); print}' >&2
    fi
    return "$ux_result"
}
