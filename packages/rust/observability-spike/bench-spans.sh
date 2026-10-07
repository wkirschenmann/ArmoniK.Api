#!/bin/sh
# The span scenarios, one process each, into results/span-cost.txt.
cd "$(dirname "$0")" || exit 1
mkdir -p results
EXE=../target/release/examples/span_cost.exe
for s in none off off2 flag on on3 poll; do
  $EXE $s
done > results/span-cost.txt 2>&1
../target/release/examples/stats_cost.exe 4 > results/stats-cost.txt 2>&1
for args in "1 0" "1 256" "1 4352" "2 4352" "8 4352"; do
  ../target/release/examples/filter_change.exe $args
done > results/filter-change.txt 2>&1
../target/release/examples/filter_change.exe 2 4352 envg >> results/filter-change.txt 2>&1
