#!/usr/bin/env python3
"""Checks that design.md's normative property lists and the manifests in the
TLA+ modules name the same properties.

Each level's list in the document is bound to the conjunctions that level
proves.  A property the document claims but no manifest carries is a promise
nothing proves; a manifest conjunct the document omits is a guarantee nobody
can find.  Both are how a specification and its document drift apart, and the
send-window accounting drifted exactly that way.

The document groups properties by topic and the manifests group them by kind,
so the comparison is per level rather than per list: a name must appear in one
of the level's manifests, not in a particular one.

Exit status 0 when every level matches, 1 otherwise.
"""

import io
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TLA = os.path.dirname(HERE)
SPEC = os.path.dirname(TLA)
DOC = os.path.join(SPEC, "design.md")

# Structural conjuncts of a manifest that the document deliberately does not
# list: they say the state is well typed, not what the library guarantees.
STRUCTURAL = {"TypeOK", "ManagedTypeOK"}

# Guarantees about an action rather than about a state, which no conjunction
# of state predicates can carry.  Each must be a declared theorem.
ACTION_GUARANTEES = {
    "Level 1": {"WriteDoneFreesASlot": "FfiGrpcTheorems.tla",
                "DestroyedRuntimeRejectsHandles": "FfiGrpcTheorems.tla"},
    "Level 0": {},
    "Level 2": {},
}

# level -> (document region start, document region end, manifests)
LEVELS = [
    ("Level 0",
     "#### Safety invariants (to be proved by TLAPS)",
     "### Level 1",
     [("AbstractGrpc.tla", "SafetyCore"),
      ("AbstractGrpc.tla", "LivenessProperties")]),
    ("Level 1",
     "Additional invariants (the FFI conjuncts",
     "#### Fairness",
     [("FfiGrpc_defs.tla", "FfiCallInv"),
      ("FfiGrpc_defs.tla", "BufferStateInv"),
      ("FfiGrpc_defs.tla", "SafetyInvariantExtras"),
      ("FfiGrpc.tla", "LivenessProperties")]),
    ("Level 2",
     "#### Level-2 safety invariants (to be proved by TLAPS)",
     "#### Held by construction",
     [("DotNetBinding_defs.tla", "ManagedSafety"),
      ("DotNetBinding_defs.tla", "ManagedLiveness")]),
]


def die(msg):
    sys.stderr.write("check_property_manifest: %s\n" % msg)
    sys.exit(2)


def read(path):
    return io.open(path, encoding="utf-8").read()


def doc_names(doc, start, end):
    """The bold head of every bullet between two markers, split on '/'.

    Returns the set and the names that head more than one bullet.  Comparing
    sets alone hides a duplicate, and a duplicate is how two descriptions of
    one property start to disagree: this found two copies of the buffer
    bullets, one of which a later edit would have left behind."""
    i = doc.find(start)
    if i < 0:
        die("document marker not found: %s" % start)
    j = doc.find(end, i + len(start))
    if j < 0:
        die("document marker not found: %s" % end)
    names = []
    for bold in re.findall(r"^\s*-\s+\*\*([^*]+)\*\*\s*:", doc[i:j], re.M):
        for part in bold.split("/"):
            part = part.strip()
            if re.match(r"^[A-Za-z][A-Za-z0-9]*$", part):
                names.append(part)
    twice = {n for n in names if names.count(n) > 1}
    return set(names), twice


def conjuncts(module, name):
    """The identifiers conjoined by a definition of the form  Name == /\\ A ..."""
    text = read(os.path.join(TLA, module))
    m = re.search(r"^%s ==\n((?:\s+/\\.*\n)+)" % re.escape(name), text, re.M)
    if not m:
        die("manifest not found: %s!%s" % (module, name))
    found = set()
    for line in m.group(1).splitlines():
        c = re.match(r"\s+/\\\s+([A-Za-z][A-Za-z0-9]*)\s*$", line)
        if c:
            found.add(c.group(1))
    return found - STRUCTURAL


def safety_invariant_extras(module, _name):
    """The level-1 conjuncts of SafetyInvariant beyond the inherited level-0
    one and FfiCallInv, whether or not they sit under the NotFailed guard."""
    text = read(os.path.join(TLA, module))
    m = re.search(r"^SafetyInvariant ==\n((?:\s+/\\.*\n)+)", text, re.M)
    if not m:
        die("manifest not found: %s!SafetyInvariant" % module)
    found = set()
    for line in m.group(1).splitlines():
        for ident in re.findall(r"\b([A-Za-z][A-Za-z0-9]*)\b", line):
            if ident not in ("NotFailed", "L0"):
                found.add(ident)
    return (found - {"SafetyInvariant", "FfiCallInv", "BufferStateInv"}
            - STRUCTURAL)


def theorem_exists(module, name):
    text = read(os.path.join(TLA, module))
    return re.search(r"^THEOREM %s\b" % re.escape(name), text, re.M) is not None


def main():
    doc = read(DOC)
    ok = True
    for level, start, end, manifests in LEVELS:
        documented, twice = doc_names(doc, start, end)
        for name in sorted(twice):
            ok = False
            print("  %s: %s heads more than one bullet" % (level, name))
        proved = set()
        for module, name in manifests:
            if name == "SafetyInvariantExtras":
                proved |= safety_invariant_extras(module, name)
            else:
                proved |= conjuncts(module, name)
        for name, module in ACTION_GUARANTEES[level].items():
            if not theorem_exists(module, name):
                ok = False
                print("  %s: %s is documented but not declared in %s"
                      % (level, name, module))
            proved.add(name)
        for name in sorted(documented - proved):
            ok = False
            print("  %s: %s is documented but in no manifest" % (level, name))
        for name in sorted(proved - documented):
            ok = False
            print("  %s: %s is in a manifest but undocumented" % (level, name))
    if ok:
        print("OK: design.md and the TLA+ manifests name the same properties.")
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
