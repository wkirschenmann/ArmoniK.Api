#!/usr/bin/env bash
# Model-checks one configuration. Verdict from the log text, never the
# clock: TLC exit codes conflate warnings and errors.
set -u
cfg="$1"
cd "$(dirname "$0")/.."
TLA2TOOLS="${TLA2TOOLS:-tla2tools.jar}"
[ -d "$TLA2TOOLS" ] && TLA2TOOLS="${TLA2TOOLS%/}/tla2tools.jar"
[ -r "$TLA2TOOLS" ] || { echo "no tla2tools.jar at $TLA2TOOLS"; exit 1; }
mkdir -p out
base="$(basename "$cfg" .cfg)"
# A configuration runs against the module of its own name when there is
# one, and against the shared <Module>_MC.tla otherwise.  Both shapes are
# in use: levels 0 and 1 put several configurations on one module, level 2
# gives each its own.  Only a module that declares an invariant can be
# checked against a configuration naming it.
if [ -r "$base.tla" ]; then mod="$base.tla"; else mod="${base%%_MC*}_MC.tla"; fi
log="out/tlc-$base.log"
# gzip halves the on-disk state pools; liveness checking writes a lot.
java -XX:+UseParallelGC -jar "$TLA2TOOLS" \
     -config "$cfg" -metadir "out/$base" -workers auto \
     -gzip -deadlock "$mod" > "$log" 2>&1
rc=$?
rm -rf "out/$base"
# A witness configuration states its target negatively, so the violation
# trace is the result and "no error" is the failure: reaching the end of
# the state space without it means the behaviour it claims is unreachable,
# and a proof about that branch would be a proof about a step that never
# fires.  Its verdict is therefore the other way round.
case "$base" in
  *witness)
    if grep -q "Error: Invariant .* is violated" "$log"; then
      grep -E "Error: Invariant|The depth" "$log" | tail -2
      echo "TLC witness ok: $cfg reached its target"
      exit 0
    fi
    echo "TLC WITNESS FAILED: $cfg never reached its target"
    tail -40 "$log"
    exit 1
    ;;
esac
if grep -q "Model checking completed. No error has been found." "$log"; then
  grep -E "distinct states|Finished in" "$log" | tail -2
  echo "TLC ok: $cfg"
  exit 0
fi
echo "TLC FAILED: $cfg"
tail -40 "$log"
exit 1
