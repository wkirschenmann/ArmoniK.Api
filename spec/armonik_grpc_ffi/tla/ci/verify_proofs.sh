#!/usr/bin/env bash
# Runs tlapm on the proofs module and reads the verdict from the log text:
# an elaboration error exits fast and must not read as success, and tlapm
# ends "abnormally" whenever any obligation is unproved.
set -u
cd "$(dirname "$0")/.."
TLAPM="${TLAPM:-tlapm}"
STRETCH="${STRETCH:-5}"
log="${TLAPM_LOG:-out/tlapm-proofs.log}"
mkdir -p "$(dirname "$log")"
"$TLAPM" --cleanfp --stretch "$STRETCH" AbstractGrpcTheorems_proofs.tla \
    > "$log" 2>&1
txt=$(tr -d '\0' < "$log")
if echo "$txt" | grep -qE "^Error:|Parse Error|not found"; then
  echo "TLAPM ELABORATION ERROR"
  echo "$txt" | grep -B2 -A8 -E "^Error:|Parse Error|not found" | head -40
  exit 1
fi
if echo "$txt" | grep -qE "All [0-9]+ obligations proved"; then
  echo "$txt" | grep -E "All [0-9]+ obligations proved"
  exit 0
fi
echo "TLAPM: obligations unproved or no summary line"
echo "$txt" | grep -B1 -E "\[ERROR\]" | head -40
echo "$txt" | tail -10
exit 1
