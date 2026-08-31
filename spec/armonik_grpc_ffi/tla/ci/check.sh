#!/usr/bin/env bash
# Fast checks: the declaration/proof consistency contract, and SANY on the
# SANY-clean modules (the proofs module is tlapm territory: it contains
# [](A => A') forms that SANY rejects but tlapm accepts).
set -u
cd "$(dirname "$0")/.."
TLA2TOOLS="${TLA2TOOLS:-tla2tools.jar}"
fail=0

# `command -v python3` finds the Windows Store stub, which exists, is on
# PATH and fails the moment it runs - so presence is not the test, and
# merely executing it can pop the Store UI.  Probe python first.
PY=""
for cand in python py python3; do
  if "$cand" -c "" >/dev/null 2>&1; then PY="$cand"; break; fi
done
[ -n "$PY" ] || { echo "no working python interpreter on PATH"; exit 1; }
"$PY" check_theorem_statements.py || fail=1

for m in AbstractGrpcState.tla AbstractGrpc.tla AbstractGrpc_defs.tla \
         AbstractGrpcTheorems.tla AbstractGrpc_MC.tla; do
  out=$(java -cp "$TLA2TOOLS" tla2sany.SANY "$m" 2>&1)
  if echo "$out" | grep -qE "Parse Error|Semantic error|Fatal errors"; then
    echo "SANY FAILED: $m"
    echo "$out" | tail -20
    fail=1
  else
    echo "SANY ok: $m"
  fi
done
exit $fail
