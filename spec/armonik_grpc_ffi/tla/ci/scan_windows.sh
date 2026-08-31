#!/usr/bin/env bash
# Verifies a line range of a proofs module window by window and prints one
# line per window. Unlike verify_proofs.sh this is a development scan: it
# keeps going after a failing window so one pass surfaces every failure.
set -u
cd "$(dirname "$0")/.."
mod="${MOD:-FfiGrpcTheorems_proofs.tla}"
TLAPM="${TLAPM:-/c/Users/wkirschenmann/TlaTools/tlapm_.cmd}"
FAILPY="${FAILPY:-}"
from="${1:-1}"
upto="${2:-$(wc -l < "$mod")}"
WINDOW="${WINDOW:-190}"
THREADS="${THREADS:-2}"
# Lower it to surface the steps sitting close to their time budget: a step that
# only just closes at the usual factor is a step that fails on a busier machine,
# and two consecutive full passes failing at two different lines is how that
# shows up. STRETCH=1 turns those into a bounded list.
STRETCH="${STRETCH:-5}"
mkdir -p out
while [ "$from" -lt "$upto" ]; do
    to=$((from + WINDOW))
    [ "$to" -gt "$upto" ] && to=$upto
    log="out/scan-$from-$to.log"
    "$TLAPM" --strict --nofp --threads "$THREADS" --stretch "$STRETCH" \
        --toolbox "$from" "$to" "$mod" > "$log" 2>&1
    txt=$(tr -d '\0' < "$log")
    if echo "$txt" | grep -q "Catastrophic failure"; then
        echo "$from-$to WSL DIED"
        exit 2
    fi
    n=$(echo "$txt" | grep -oE "All [0-9]+ obligations? proved" | tail -1)
    if [ -n "$n" ]; then
        echo "$from-$to  $n"
    else
        echo "$from-$to  FAILED"
        [ -n "$FAILPY" ] && python "$FAILPY" "$log"
        echo "$txt" | grep -E "^Error:|Parse Error" | head -5
    fi
    from=$to
done
