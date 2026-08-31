# Checks the AbstractGrpcTheorems / AbstractGrpcTheorems_proofs contract:
# every theorem declared in the interface must be restated verbatim in the
# proofs module. Run it whenever either file changes; CI should run it
# before tlapm.
import io
import os
import re
import sys

D = os.path.dirname(os.path.abspath(__file__))


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


iface = statements(os.path.join(D, "AbstractGrpcTheorems.tla"))
proofs = statements(os.path.join(D, "AbstractGrpcTheorems_proofs.tla"))

bad = False
for name, stmt in iface.items():
    if name not in proofs:
        print("MISSING IN PROOFS:", name)
        bad = True
    elif proofs[name] != stmt:
        print("STATEMENT DRIFT:", name)
        bad = True
if bad:
    sys.exit(1)
print("OK: %d declarations, all restated verbatim in the proofs module."
      % len(iface))
