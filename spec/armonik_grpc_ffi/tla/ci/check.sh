#!/usr/bin/env bash
# Fast checks: the declaration/proof consistency contract, the binding
# between design.md's property lists and the manifests, and SANY on the
# SANY-clean modules (the proofs module is tlapm territory: it contains
# [](A => A') forms that SANY rejects but tlapm accepts).
set -u
cd "$(dirname "$0")/.."
TLA2TOOLS="${TLA2TOOLS:-tla2tools.jar}"
# A directory here is a common way to set it; accept it rather than reporting
# every module clean because the launcher never ran.
[ -d "$TLA2TOOLS" ] && TLA2TOOLS="${TLA2TOOLS%/}/tla2tools.jar"
[ -r "$TLA2TOOLS" ] || { echo "no tla2tools.jar at $TLA2TOOLS"; exit 1; }
fail=0

# Pick an interpreter that actually runs.  `command -v python3` finds the
# Windows Store stub, which exists, is on PATH, and fails with "Permission
# denied" the moment it is executed - so presence is not the test.  Probe
# python first: merely executing the stub can pop the Store UI, so python3
# must be the last resort, not the first probe.
PY=""
for cand in python py python3; do
  if "$cand" -c "" >/dev/null 2>&1; then PY="$cand"; break; fi
done
[ -n "$PY" ] || { echo "no working python interpreter on PATH"; exit 1; }
"$PY" ci/check_theorem_statements.py || fail=1
"$PY" ci/check_action_footprints.py || fail=1
"$PY" ci/check_property_manifest.py || fail=1
"$PY" ci/check_abi_coverage.py || fail=1
"$PY" ci/check_proofs_present.py || fail=1
"$PY" ci/check_arity.py || fail=1

for m in AbstractGrpcState.tla AbstractGrpc.tla AbstractGrpc_defs.tla \
         AbstractGrpcTheorems.tla AbstractGrpc_MC.tla \
         FfiGrpcState.tla FfiGrpc.tla FfiGrpc_defs.tla \
         FfiGrpcTheorems.tla FfiGrpcEnabledTheorems.tla FfiGrpc_MC.tla \
         DotNetBindingState.tla DotNetBinding.tla \
         DotNetBinding_defs.tla DotNetBindingTheorems.tla \
         DotNetBinding_MC.tla DotNetBinding_MCdirected.tla \
         DotNetBinding_MCcall.tla DotNetBinding_MClive.tla \
         DotNetBinding_MCwitness.tla; do
  out=$(java -cp "$TLA2TOOLS" tla2sany.SANY "$m" 2>&1)
  # Positive evidence, not the absence of an error word: a launcher failure
  # matches no error pattern, and reporting that as clean is worse than no
  # check at all.  SANY always announces the module it parsed.
  if ! echo "$out" | grep -q "Semantic processing of module ${m%.tla}$"; then
    echo "SANY DID NOT RUN: $m"
    echo "$out" | tail -5
    fail=1
  elif echo "$out" | grep -qE "Parse Error|Semantic error|Fatal errors"; then
    echo "SANY FAILED: $m"
    echo "$out" | tail -20
    fail=1
  else
    echo "SANY ok: $m"
  fi
done
exit $fail
