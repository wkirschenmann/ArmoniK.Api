#!/usr/bin/env python3
"""Checks that the proofs modules apply each operator with the arity it is
defined with.

SANY would find this in a second, and check.sh runs it - but not on the proofs
modules, which contain [](A => A') forms SANY rejects.  So the largest files in
the specification have no cheap syntactic check at all, and a stale arity there
surfaces only as tlapm printing "Invalid number of arguments" for a whole
window, naming no line.  Changing SendMessage(cId, msg) to take the buffer it
commits left one such application behind, and that is what it cost to find.

Only operators defined in the specification modules are checked, and only
applications written without a module prefix: an operator reached through
L0! belongs to another module and its arity is that module's business.

Exit status 0 when every application matches, 1 otherwise.
"""

import io
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TLA = os.path.dirname(HERE)

# (proofs module, the modules whose definitions it applies)
PAIRS = [
    ("FfiGrpcTheorems_proofs.tla",
     ["FfiGrpcState.tla", "FfiGrpc.tla", "FfiGrpc_defs.tla",
      "FfiGrpcTheorems.tla", "FfiGrpcTheorems_proofs.tla"]),
    ("AbstractGrpcTheorems_proofs.tla",
     ["AbstractGrpcState.tla", "AbstractGrpc.tla", "AbstractGrpc_defs.tla",
      "AbstractGrpcTheorems.tla", "AbstractGrpcTheorems_proofs.tla"]),
]

# Bound identifiers shadow a definition of the same name, and a proof step is
# free to use a one-letter operator parameter.  Definitions of arity zero are
# not applied with parentheses at all, so they are not checked.
DEF = re.compile(r"^([A-Za-z]\w*)\(([^)]*)\)\s*==", re.M)

OPEN = {"(": ")", "[": "]", "{": "}"}


def strip_comments(text):
    text = re.sub(r"\(\*.*?\*\)", " ", text, flags=re.S)
    return re.sub(r"\\\*.*$", "", text, flags=re.M)


def read(name):
    path = os.path.join(TLA, name)
    if not os.path.exists(path):
        return None
    return strip_comments(io.open(path, encoding="utf-8").read())


def arities(modules):
    """name -> declared number of parameters, for parameterised operators."""
    out = {}
    for m in modules:
        text = read(m)
        if text is None:
            continue
        for d in DEF.finditer(text):
            params = [p for p in d.group(2).split(",") if p.strip()]
            out[d.group(1)] = len(params)
    return out


def top_level_commas(text, i):
    """Walk the argument list starting just after '(' at text[i].

    Returns (count, end) where count is the number of top-level arguments and
    end indexes the matching ')'.  Returns (None, None) if unbalanced, which
    happens when a regex match starts inside a larger expression."""
    depth = []
    args = 1
    empty = True
    j = i
    n = len(text)
    while j < n:
        c = text[j]
        # A delimiter is content: the one argument of Foo({}) is the empty
        # set, and only Foo() has no argument at all.
        if not c.isspace() and not (c == ")" and not depth):
            empty = False
        if text.startswith("<<", j):
            depth.append(">>")
            j += 2
            continue
        if text.startswith(">>", j):
            if depth and depth[-1] == ">>":
                depth.pop()
            j += 2
            continue
        if c in OPEN:
            depth.append(OPEN[c])
        elif c in (")", "]", "}"):
            if not depth:
                if c != ")":
                    return None, None
                return (0 if empty else args), j
            if depth[-1] != c:
                return None, None
            depth.pop()
        elif c == "," and not depth:
            args += 1
        j += 1
    return None, None


def main():
    ok = True
    for proofs, modules in PAIRS:
        text = read(proofs)
        if text is None:
            continue
        known = arities(modules)
        if not known:
            continue
        names = "|".join(sorted(known, key=len, reverse=True))
        # A '!' before the name means another module; a word character before
        # it means this is the tail of a longer identifier.
        pattern = re.compile(r"(?<![\w!])(" + names + r")\(")
        seen = {}
        for m in pattern.finditer(text):
            name = m.group(1)
            count, end = top_level_commas(text, m.end())
            if count is None:
                continue
            want = known[name]
            if count != want:
                line = text.count("\n", 0, m.start()) + 1
                key = (name, count, want)
                if key in seen:
                    continue
                seen[key] = line
                ok = False
                print("  %s:%d: %s takes %d argument%s, applied with %d"
                      % (proofs, line, name, want,
                         "" if want == 1 else "s", count))
    if ok:
        print("OK: every application matches its definition's arity.")
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
