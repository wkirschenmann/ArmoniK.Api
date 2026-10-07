#!/bin/sh
# Every front over the cost scenarios, into results/cost-<front>.txt.
cd "$(dirname "$0")" || exit 1
mkdir -p results
for front in "$@"; do
  sh bench-cost.sh "$front" > "results/cost-$front.txt" 2>&1
done
