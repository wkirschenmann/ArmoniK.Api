#!/usr/bin/env python3
"""Conservative dead-lemma detector for the ArmoniK gRPC FFI proof corpus.

Run it after every structural refactor: helpers go stale silently when a
transfer lemma or a frame supersedes them, and a proof file only grows.

Method: over-approximate the citation graph - an edge L -> M exists when
the NAME M appears anywhere in L's proof body, comments stripped. Over-
approximating the edges under-approximates the dead set, so the tool never
flags a live lemma. The graph PROPOSES; re-verification DISPOSES: remove a
candidate, re-run tlapm, and an elaboration error names what was removed in
error.

Roots are not a hand-written list here. Each level declares its citable
surface in a *Theorems.tla module, and check_theorem_statements.py already
enforces that every declaration is restated verbatim in the proofs module.
So the declarations ARE the roots, and the two tools stay in agreement by
construction.
"""
import io
import os
import re
import sys
from collections import defaultdict

HERE = os.path.dirname(os.path.abspath(__file__))
DIR = sys.argv[1] if len(sys.argv) > 1 else HERE

PAIRS = [("AbstractGrpcTheorems.tla", "AbstractGrpcTheorems_proofs.tla"),
         ("FfiGrpcTheorems.tla", "FfiGrpcTheorems_proofs.tla")]

name_re = re.compile(r'^(LEMMA|THEOREM)\s+([A-Za-z0-9_]+)\s*==')
# A body ends at a module-level boundary: a keyword, a separator, or a
# column-0 operator definition - nullary AND parameterised. Missing the
# parameterised form swallows the next operator into the body, and removing
# the lemma then deletes an operator that is still cited.
top_re = re.compile(r'^(LEMMA|THEOREM|AXIOM|====|----'
                    r'|[A-Za-z0-9_]+\s*==|[A-Za-z0-9_]+\s*\([^)]*\)\s*==)')


def strip_comments(s):
    """Drop TLA+ comments. A name merely mentioned in documentation is not a
    dependency; counting those over-connects the graph and hides real dead
    code."""
    out, i, depth, n = [], 0, 0, len(s)
    while i < n:
        if depth == 0 and s.startswith("(*", i):
            depth, i = 1, i + 2
        elif depth > 0:
            if s.startswith("(*", i):
                depth, i = depth + 1, i + 2
            elif s.startswith("*)", i):
                depth, i = depth - 1, i + 2
            else:
                if s[i] == "\n":
                    out.append("\n")
                i += 1
        elif s.startswith("\\*", i):
            j = s.find("\n", i)
            i = n if j < 0 else j
        else:
            out.append(s[i])
            i += 1
    return "".join(out)


def read(path):
    return io.open(path, encoding="utf-8", errors="replace").read().split("\n")


def analyse(decls, proofs):
    """One level at a time. The two levels share lemma names (each proves its
    own TypeOKSplit, its own frames); reading them into one table lets the
    second silently overwrite the first, which corrupts the graph and hides
    every dead lemma behind a phantom edge."""
    dpath, ppath = os.path.join(DIR, decls), os.path.join(DIR, proofs)
    if not (os.path.exists(dpath) and os.path.exists(ppath)):
        return
    roots = {m.group(2) for m in (name_re.match(l) for l in read(dpath)) if m}
    defs, bodies = {}, {}
    lines = read(ppath)
    i = 0
    while i < len(lines):
        m = name_re.match(lines[i])
        if m:
            nm, j = m.group(2), i + 1
            while j < len(lines) and not top_re.match(lines[j]):
                j += 1
            defs[nm] = i + 1
            bodies[nm] = strip_comments("\n".join(lines[i:j]))
            i = j
        else:
            i += 1
    names = set(defs)
    if not names:
        print("%s: no lemmas found" % proofs)
        return
    allnames_re = re.compile(
        r'\b(' + '|'.join(sorted(map(re.escape, names), key=len, reverse=True)) + r')\b')
    edges = defaultdict(set)
    for L in names:
        for M in set(allnames_re.findall(bodies[L])):
            if M != L:
                edges[L].add(M)
    missing = roots - names
    roots &= names
    seen, stack = set(), list(roots)
    while stack:
        n = stack.pop()
        if n in seen:
            continue
        seen.add(n)
        stack.extend(edges[n] - seen)
    dead = names - seen
    print("\n=== %s" % proofs)
    print("lemmas and theorems: %d   roots: %d   live: %d   DEAD: %d"
          % (len(names), len(roots), len(seen), len(dead)))
    if missing:
        print("DECLARED BUT NOT PROVED HERE: %s" % sorted(missing))
    for ln, nm in sorted((defs[nm], nm) for nm in dead):
        cites = sum(1 for L in names if nm in edges[L])
        print("  %6d  %s   (cited-by=%d)" % (ln, nm, cites))


for pair in PAIRS:
    analyse(*pair)
sys.exit(0)
