#!/usr/bin/env python3
"""Checks that every action the document's implementation sketches cite is an
action the specification actually has.

The sketches are normative: they say what the code must do, step by step, and
each step names the action of the machine it realizes in a `// TLA: ...`
comment.  That marker is what makes a sketch auditable at all.  Prose read
against prose does not catch an ordering that differs from the machine's -
two reviews of one sketch called it consistent while it released a payload
before the read's result was decided, a state the machine does not have - but
a name that no longer exists is mechanical, and a citation pointing at
nothing is the first sign that a sketch and the machine have parted.

This does not check that the ORDER matches; nothing short of a proof does.
It checks that the citations are real, so a reader who follows them arrives
somewhere, and that renaming an action cannot leave a sketch behind.

Exit status 0 when every cited action exists, 1 otherwise.
"""

import io
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TLA = os.path.dirname(HERE)
DOC = os.path.join(os.path.dirname(TLA), "design.md")

# A citation: "// TLA: Name", "// TLA: A, B", "// TLA: A against B", and so
# on.  Matched anywhere in a comment rather than only at its start, so a
# citation cannot escape by sitting mid-sentence - an instrument that
# formatting can evade is worth nothing, which is how the first version of
# this checker reported seven citations while missing three.
CITE = re.compile(r"//.*?\bTLA:\s*(.+?)(?:\.\s*$|$)", re.MULTILINE)
NAME = re.compile(r"\b([A-Z][A-Za-z0-9_]*)\b")

# A fenced code block, which is where a sketch lives.
FENCE = re.compile(r"^```[^\n]*\n(.*?)^```", re.MULTILINE | re.DOTALL)

# A defined operator, in any module of the tree.
DEFN = re.compile(r"^([A-Za-z][A-Za-z0-9_]*)\s*(?:\([^)]*\))?\s*==",
                  re.MULTILINE)


def main():
    if not os.path.isfile(DOC):
        print("check_sketch_actions: no design.md at %s" % DOC)
        return 1

    defined = set()
    for f in os.listdir(TLA):
        if not f.endswith(".tla") or "_TTrace_" in f:
            continue
        body = io.open(os.path.join(TLA, f), encoding="utf-8").read()
        defined.update(DEFN.findall(body))
    if not defined:
        print("check_sketch_actions: no definition found in %s" % TLA)
        return 1

    doc = io.open(DOC, encoding="utf-8").read()
    # Only inside fenced code blocks: prose that mentions the marker is
    # discussing it, not citing an action, and a checker that reads its own
    # documentation as input fails on it.
    cites, bad = 0, []
    for fence in FENCE.finditer(doc):
        body, base = fence.group(1), fence.start(1)
        for m in CITE.finditer(body):
            line = doc.count("\n", 0, base + m.start()) + 1
            for name in NAME.findall(m.group(1)):
                cites += 1
                if name not in defined:
                    bad.append((line, name))

    if not cites:
        print("check_sketch_actions: no sketch cites an action - the "
              "`// TLA:` markers are how a sketch stays auditable")
        return 1

    if bad:
        print("Sketches citing actions the specification does not define:")
        for line, name in bad:
            print("  design.md:%d: %s" % (line, name))
        return 1

    print("OK: %d action citations in the sketches, all defined." % cites)
    return 0


if __name__ == "__main__":
    sys.exit(main())
