#!/bin/sh
# Runs the cost example over the scenarios of the spike report; one process per scenario.
# Usage: sh bench-cost.sh <front> [extra env, e.g. SENTINEL=1]
EXE="$(dirname "$0")/../target/release/examples/cost.exe"
FRONT="$1"
run() { "$EXE" "$FRONT" "$@" 10000000 15; }

run - - debug              # no dispatcher at all
run - info debug           # a process-wide subscriber alone, the event below its filter
run info - debug           # one runtime, the event below its filter
run info+info - debug      # two runtimes, same filter
run info+debug - debug     # two runtimes, this thread's filter excludes what the other's enables
run info+debug warn debug  # the same, and a process-wide subscriber of its own
run info+info+info+info - debug
run debug - debug          # one runtime, the event enabled for it
run info+debug - debug
run info - info            # one runtime, an event it enables
run info+debug - info
