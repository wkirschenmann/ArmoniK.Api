#!/usr/bin/env python3
"""Checks that every declared result is restated verbatim in its proofs module.

The declarations module is the citable interface: level 2 reads it alone to know
what holds. The proofs module restates each declaration and discharges it. If the
two drift, the interface promises something the proof never established - and
nothing else catches it, because each module parses and proves on its own.

Verbatim means: same statement modulo comments and whitespace. A declaration
missing from the proofs module, or present with a different statement, fails.
A result proved but not declared is not an error - the proofs module has its own
lemmas, and only what it exports has to match.

Exit status 0 when every declaration matches, 1 otherwise.
"""

import io
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TLA = os.path.dirname(HERE)

PAIRS = [
    ("FfiGrpcTheorems.tla", "FfiGrpcTheorems_proofs.tla"),
    ("AbstractGrpcTheorems.tla", "AbstractGrpcTheorems_proofs.tla"),
]

HEAD = re.compile(r"^(THEOREM|LEMMA|PROPOSITION|COROLLARY)\s+(\w+)\s*==(.*)$")
# What ends a statement: the next declaration, a proof, a comment at column 0,
# a block comment, the module footer, or a blank line.
PROOF = re.compile(r"^(<\d+>|BY\b|OBVIOUS\b|OMITTED\b|PROOF\b|USE\b|ASSUME\b)")
ENDER = re.compile(r"^(\\\*|\(\*|=====|\s*$)")


def statements(path):
    """Maps each declared name to its normalized statement text."""
    lines = io.open(path, encoding="utf-8").read().splitlines()
    out = {}
    i = 0
    while i < len(lines):
        m = HEAD.match(lines[i])
        if not m:
            i += 1
            continue
        name, body = m.group(2), [m.group(3)]
        i += 1
        while i < len(lines):
            line = lines[i]
            if HEAD.match(line) or PROOF.match(line) or ENDER.match(line):
                break
            body.append(line)
            i += 1
        text = " ".join(body)
        text = re.sub(r"\\\*.*", "", text)
        out[name] = " ".join(text.split())
    return out


def main():
    failures = []
    total = 0
    for decls, proofs in PAIRS:
        dpath, ppath = os.path.join(TLA, decls), os.path.join(TLA, proofs)
        if not (os.path.exists(dpath) and os.path.exists(ppath)):
            failures.append("%s / %s: module missing" % (decls, proofs))
            continue
        declared, proved = statements(dpath), statements(ppath)
        total += len(declared)
        for name in sorted(declared):
            if name not in proved:
                failures.append("%s: %s declared, never restated in %s"
                                % (decls, name, proofs))
            elif declared[name] != proved[name]:
                failures.append(
                    "%s: %s restated differently in %s\n  declared: %s\n  proved:   %s"
                    % (decls, name, proofs, declared[name], proved[name]))
    if failures:
        for f in failures:
            print("FAIL: " + f)
        print("FAIL: %d declaration(s) do not match their proof." % len(failures))
        return 1
    print("OK: %d declarations, each restated verbatim in its proofs module."
          % total)
    return 0


if __name__ == "__main__":
    sys.exit(main())
