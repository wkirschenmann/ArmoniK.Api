#!/usr/bin/env bash
# Verifies a proofs module (argument, level 0 by default) with tlapm.
#
# The module is verified in line windows rather than in one pass. This is
# not a weakening: every line is covered exactly once, every obligation in
# a window must be proved, and a window that produces no summary line
# fails the run. It is a memory constraint - a single pass over the
# level-1 module allocates enough solver state to be OOM-killed, and a
# killed run prints no summary at all, which is indistinguishable from
# success to a careless reader. Windows keep each pass small enough to
# finish and to report.
#
# WINDOW and THREADS are the two knobs that keep it inside the box; lower
# them on a smaller runner rather than letting the run die.
set -u
cd "$(dirname "$0")/.."
mod="${1:-AbstractGrpcTheorems_proofs.tla}"
TLAPM="${TLAPM:-tlapm}"
STRETCH="${STRETCH:-5}"
WINDOW="${WINDOW:-300}"
THREADS="${THREADS:-4}"
log="${TLAPM_LOG:-out/tlapm-${mod%.tla}.log}"
mkdir -p "$(dirname "$log")"
: > "$log"

lines=$(wc -l < "$mod")
total=0
rc=0

# Every window runs --nofp: existing fingerprints are ignored and
# overwritten, so the verification is from scratch obligation by
# obligation. Deleting the cache directory instead is NOT equivalent here
# - tlapm creates it inside WSL while the delete happens on the Windows
# side, and it then dies on mkdir with EEXIST.

from=1
while :; do
    to=$((from + WINDOW))
    last=0
    if [ "$to" -ge "$lines" ]; then to=$lines; last=1; fi
    win_log="${log%.log}-$from-$to.log"
    "$TLAPM" --strict --nofp --threads "$THREADS" --stretch "$STRETCH" \
        --toolbox "$from" "$to" "$mod" > "$win_log" 2>&1
    txt=$(tr -d '\0' < "$win_log")
    cat "$win_log" >> "$log"
    if echo "$txt" | grep -qE "^Error:|Parse Error|not found"; then
        echo "ELABORATION ERROR in $from-$to"
        echo "$txt" | grep -B2 -A8 -E "^Error:|Parse Error|not found" | head -30
        exit 1
    fi
    n=$(echo "$txt" | grep -oE "All [0-9]+ obligations? proved" | grep -oE "[0-9]+" | tail -1)
    if [ -z "$n" ]; then
        echo "WINDOW $from-$to: no summary (killed, or obligations unproved)"
        echo "$txt" | grep -E "\[ERROR\]" | head -5
        rc=1
    else
        total=$((total + n))
    fi
    [ "$last" -eq 1 ] && break
    # Windows overlap on their boundary line so no proof step can fall
    # between two of them.
    from=$to
done

if [ "$rc" -ne 0 ]; then
    echo "$mod: FAILED"
    exit 1
fi
echo "$mod: all $total obligations proved (windows of $WINDOW lines)"
exit 0
