#!/usr/bin/env python3
"""Checks that every derived invariant is named in design.md's list of them.

A derived invariant is one a level-2 theorem carries as `Spec => []Inv`
without it being a manifest conjunct: a fact about the machine that no safety
statement asked for and a leads-to edge cannot do without.  The manifest
checker cannot see them - it binds the document to the manifests - so the
document's list is the only thing that binds them, and nothing was checking
that list.

It was written from one theorem when six carry such invariants, so it named
seven of thirteen the day it was added.  That is the failure this check
exists for: the set is defined by a shape, and reading the shape is the only
way to keep a hand-written list honest.

Exit status 0 when the two agree, 1 otherwise.
"""

import io
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TLA = os.path.dirname(HERE)
SPEC = os.path.dirname(TLA)
DOC = os.path.join(SPEC, "design.md")
PROOFS = os.path.join(TLA, "DotNetBindingTheorems_proofs.tla")
DEFS = os.path.join(TLA, "DotNetBinding_defs.tla")

# The section that lists them, and where it ends.
SECTION = "#### The derived invariants"

# Aggregates, not derived invariants: each is the conjunction the manifest
# checker already binds, or the inductive invariant carrying it.
AGGREGATES = {"ManagedIndInv", "ManagedTypeOK", "ManagedSafety",
              "ManagedLiveness"}


def manifest_names():
    """Every conjunct of the level-2 manifests, which are bound elsewhere."""
    src = io.open(DEFS, encoding="utf-8").read()
    out = set()
    for man in ("ManagedSafety", "ManagedLiveness"):
        m = re.search(r'^%s ==(.*?)(?=\n\n)' % man, src, re.S | re.M)
        if m:
            out |= set(re.findall(r'/\\\s*([A-Za-z][A-Za-z0-9_]*)', m.group(1)))
    return out


def carried():
    """invariant -> the theorems that carry it as Spec => []invariant."""
    src = io.open(PROOFS, encoding="utf-8").read()
    out = {}
    for m in re.finditer(
            r'^THEOREM\s+(\w+)\s*==\s*(.*?)(?=\n<|\nBY|\nOBVIOUS|\Z)',
            src, re.S | re.M):
        if "Spec =>" not in m.group(2):
            continue
        # not an instance-prefixed name: []L1!Something is level 1's,
        # carried here by the refinement rather than derived here
        for inv in re.findall(r'\[\]([A-Za-z][A-Za-z0-9_]*)(?![\w!])',
                              m.group(2)):
            out.setdefault(inv, []).append(m.group(1))
    return out


def listed():
    """The names design.md's derived-invariants table gives, in order."""
    doc = io.open(DOC, encoding="utf-8").read()
    if SECTION not in doc:
        return None
    body = doc[doc.index(SECTION) + len(SECTION):]
    end = re.search(r'^#{2,4} ', body, re.M)
    if end:
        body = body[: end.start()]
    return [m.group(1) for m in
            re.finditer(r'^\| `([A-Za-z][A-Za-z0-9_]*)` \|', body, re.M)]


def main():
    names = listed()
    if names is None:
        print('MISSING: design.md has no "%s" section' % SECTION)
        return 1
    skip = AGGREGATES | manifest_names()
    derived = {i: t for i, t in carried().items() if i not in skip}
    ok = True
    for inv in sorted(derived):
        if inv not in names:
            print("NOT LISTED: %s, carried by %s"
                  % (inv, ", ".join(sorted(derived[inv]))))
            ok = False
    for name in names:
        if name not in derived:
            print("LISTED BUT NOT CARRIED: %s is in the document's table and "
                  "no theorem states Spec => []%s" % (name, name))
            ok = False
    if ok:
        print("OK: %d derived invariants, each listed in design.md."
              % len(derived))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
