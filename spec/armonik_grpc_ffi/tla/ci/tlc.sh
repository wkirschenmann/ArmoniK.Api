#!/usr/bin/env bash
# Model-checks one configuration. Verdict from the log text, never the
# clock: TLC exit codes conflate warnings and errors.
set -u
cfg="$1"
cd "$(dirname "$0")/.."
TLA2TOOLS="${TLA2TOOLS:-tla2tools.jar}"
mkdir -p out
base="$(basename "$cfg" .cfg)"
# <Module>_MC*.cfg runs against <Module>_MC.tla
mod="${base%%_MC*}_MC.tla"
log="out/tlc-$base.log"
# gzip halves the on-disk state pools; liveness checking writes a lot.
java -XX:+UseParallelGC -jar "$TLA2TOOLS" \
     -config "$cfg" -metadir "out/$base" -workers auto \
     -gzip -deadlock "$mod" > "$log" 2>&1
rc=$?
rm -rf "out/$base"
if grep -q "Model checking completed. No error has been found." "$log"; then
  grep -E "distinct states|Finished in" "$log" | tail -2
  echo "TLC ok: $cfg"
  exit 0
fi
echo "TLC FAILED: $cfg"
tail -40 "$log"
exit 1
