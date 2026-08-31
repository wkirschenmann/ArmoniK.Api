# Checks the Theorems / Theorems_proofs contract of every level: each
# theorem declared in an interface module must be restated verbatim in its
# proofs module. Run it whenever either file changes; CI should run it
# before tlapm.
import io
import os
import re
import sys

D = os.path.dirname(os.path.abspath(__file__))

PAIRS = [
    ("AbstractGrpcTheorems.tla", "AbstractGrpcTheorems_proofs.tla"),
    ("FfiGrpcTheorems.tla", "FfiGrpcTheorems_proofs.tla"),
]


def statements(path):
    src = io.open(path, encoding="utf-8").read()
    out = {}
    for m in re.finditer(r"^THEOREM ([A-Za-z0-9_]+) ==", src, re.M):
        name = m.group(1)
        lines = src[m.start():].split("\n")
        body = [lines[0]]
        for line in lines[1:]:
            # A statement is a contiguous block: the first blank line or
            # proof token ends it.
            if not line.strip():
                break
            if re.match(r"^<\d>|^ *BY |^ *OBVIOUS|^ *OMITTED|^=====", line):
                break
            body.append(line)
        out[name] = "\n".join(body)
    return out


bad = False
total = 0
for iface_name, proofs_name in PAIRS:
    iface_path = os.path.join(D, iface_name)
    proofs_path = os.path.join(D, proofs_name)
    if not os.path.exists(proofs_path):
        print("SKIPPED (no proofs module yet): %s" % proofs_name)
        continue
    iface = statements(iface_path)
    proofs = statements(proofs_path)
    for name, stmt in iface.items():
        if name not in proofs:
            print("MISSING IN PROOFS: %s (%s)" % (name, iface_name))
            bad = True
        elif proofs[name] != stmt:
            print("STATEMENT DRIFT: %s (%s)" % (name, iface_name))
            bad = True
    total += len(iface)
if bad:
    sys.exit(1)
print("OK: %d declarations, all restated verbatim in their proofs modules."
      % total)
