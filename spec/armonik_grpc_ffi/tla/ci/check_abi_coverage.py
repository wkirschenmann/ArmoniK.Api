#!/usr/bin/env python3
"""Checks that every acting ABI function appears in the level-1 mapping table,
and that every function the table names exists in the ABI.

An acting function changes state the model carries, so it has a linearization
point and the table is where that point is recorded - the table calls itself
the contract between the proof and the code.  A function missing from it is a
piece of the surface nothing is proved about; a function the table names but
the ABI does not declare is a proof anchored to something that does not exist.
Both were present when this check was written: the table named ak_channel_close
for what the ABI declares as ak_channel_release.

Purely observational functions are exempt and listed here by name, so adding
one is a deliberate act rather than an omission.

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

# Functions that read and change nothing: they have no linearization point
# because they do not linearize.  ak_call_debt_of reports what a call still
# owes, ak_runtime_status reports a state, ak_abi_version is a constant.
OBSERVATIONAL = {
    "ak_abi_version",
    "ak_runtime_status",
    "ak_call_debt_of",
    "ak_runtime_memory_usage",
    "ak_runtime_memory_usage_detailed",
}

TABLE_START = "### Where each level-1 action happens"
TABLE_END = "### JSON configuration schema"

# The argument table lives inside the same region and records, per ABI
# argument, what it becomes in the model or why it does not.  Every argument
# name of every acting function must appear there.
ARG_TABLE = "#### Which ABI argument becomes what"

# Argument names the table covers by class rather than one by one.  The value
# is the token the table must contain, backticks included: a bare substring
# test has no teeth, because names like len sit inside ordinary words.
ARG_CLASSES = {
    "runtime": "`ak_*_handle`",
    "channel": "`ak_*_handle`",
    "call": "`ak_*_handle`",
    "out": "`*out`",
}


def die(msg):
    sys.stderr.write("check_abi_coverage: %s" % msg + chr(10))
    sys.exit(2)


def read(path):
    return io.open(path, encoding="utf-8").read()


def declared_functions(doc):
    """Every ak_* function the ABI blocks declare."""
    out = set()
    for m in re.finditer(r"^(?:ak_status|void|int|ak_runtime_state)\s+(ak_\w+)\s*\(",
                         doc, re.M):
        out.add(m.group(1))
    return out


def declared_arguments(doc):
    """function -> the bare names of its parameters."""
    out = {}
    for m in re.finditer(
            r"^(?:ak_status|void|int|ak_runtime_state)\s+(ak_\w+)\s*\(([^;]*)\);",
            doc, re.M | re.S):
        args = []
        for part in m.group(2).split(","):
            part = part.strip()
            if not part or part == "void":
                continue
            name = re.findall(r"(\w+)\s*$", part.replace("*", " "))
            if name:
                args.append(name[0])
        out[m.group(1)] = args
    return out


def table_region(doc):
    i = doc.find(TABLE_START)
    if i < 0:
        sys.stderr.write("check_abi_coverage: mapping table not found\n")
        sys.exit(2)
    j = doc.find(TABLE_END, i)
    return doc[i:j if j > 0 else len(doc)]


def main():
    doc = read(DOC)
    declared = declared_functions(doc)
    table = table_region(doc)
    named = set(re.findall(r"ak_\w+", table))

    ok = True
    i = doc.find(ARG_TABLE)
    if i < 0:
        die("argument table not found")
    rest = doc[i + len(ARG_TABLE):]
    # The region is the table itself: everything up to the next heading of
    # any level.  Running to the next "#### " swallowed half the document,
    # which is how a removed row went unnoticed.
    rows = []
    for line in rest.split("\n"):
        if line.startswith("#"):
            break
        rows.append(line)
    args_region = "\n".join(rows)
    for fn, args in declared_arguments(doc).items():
        if fn in OBSERVATIONAL:
            continue
        for a in args:
            key = ARG_CLASSES.get(a, "`" + a + "`")
            if key not in args_region:
                ok = False
                print("  %s's argument %s is in no row of the argument table"
                      % (fn, a))
    for fn in sorted(declared - named - OBSERVATIONAL):
        ok = False
        print("  %s is declared in the ABI but has no row in the mapping table"
              % fn)
    for fn in sorted(named - declared):
        ok = False
        print("  the mapping table names %s, which the ABI does not declare"
              % fn)
    for fn in sorted(OBSERVATIONAL - declared):
        ok = False
        print("  %s is listed as observational but the ABI does not declare it"
              % fn)
    if ok:
        print("OK: every acting ABI function has a linearization point.")
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
