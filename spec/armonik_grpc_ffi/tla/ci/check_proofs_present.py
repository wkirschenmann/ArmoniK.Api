#!/usr/bin/env python3
"""Checks that every theorem and lemma of a proofs module carries a proof.

A theorem written without a proof body parses, and tlapm reports no failed
obligation for it because it generates none.  So it is invisible twice over:
greping for OMITTED does not find it, and a green run does not contradict it.
Two liveness theorems sat in that state, stated and cited by the top-level
conjunction, while nothing at all had been proved about them.

A proof is a first-level step, an OBVIOUS, or a BY.  OMITTED is reported
separately: it is an explicit hole rather than an invisible one, but the rule
here is that there are none.

Exit status 0 when every declaration is proved, 1 otherwise.
"""

import io
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TLA = os.path.dirname(HERE)

MODULES = [
    "FfiGrpcTheorems_proofs.tla",
    "AbstractGrpcTheorems_proofs.tla",
]

HEAD = re.compile(r"^(THEOREM|LEMMA|PROPOSITION|COROLLARY)\s+(\w+)\s*==", re.M)
END = re.compile(r"^(?:THEOREM|LEMMA|PROPOSITION|COROLLARY)\s+\w+\s*=="
                 r"|^=====", re.M)


def strip_comments(text):
    text = re.sub(r"\(\*.*?\*\)", " ", text, flags=re.S)
    return re.sub(r"\\\*.*$", "", text, flags=re.M)


def main():
    ok = True
    for name in MODULES:
        path = os.path.join(TLA, name)
        if not os.path.exists(path):
            continue
        text = strip_comments(io.open(path, encoding="utf-8").read())
        heads = list(HEAD.finditer(text))
        for k, m in enumerate(heads):
            start = m.end()
            stop = heads[k + 1].start() if k + 1 < len(heads) else len(text)
            end = END.search(text, start, stop)
            body = text[start:end.start() if end else stop]
            if "OMITTED" in body:
                ok = False
                print("  %s: %s is OMITTED" % (name, m.group(2)))
                continue
            # The statement itself may span lines, so a BY or OBVIOUS anywhere
            # after it counts; what does not count is nothing at all.
            if not re.search(r"^\s*(<1>|OBVIOUS|BY)\b", body, re.M):
                ok = False
                print("  %s: %s carries no proof" % (name, m.group(2)))
    if ok:
        print("OK: every declaration carries a proof.")
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
