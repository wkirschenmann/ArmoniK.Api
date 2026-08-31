#!/usr/bin/env python3
"""Reports the proof steps whose time budget leaves no margin.

tlapm scales every backend timeout by --stretch. A step that closes inside its
budget at the factor the gate uses can still fail at a lower one, or on a busy
machine at the same one, and then a green run and a red run differ by nothing
anyone can see. The remedy is to name the budget on the step - SMTT(n), IsaT(n)
- and the question this answers is which steps need it.

Input: the logs of a `tlapm --toolbox` run, which record for each obligation the
backend that closed it, the limit it had and the time it used. Output: one line
per step under the requested margin, addressed as `Theorem:1.2.3` - the step
path, not a line number, because a line number stops being true as soon as
anything above it is edited.

Usage:
    obligation_margin.py <module.tla> <log>... [--margin 5] [--stretch 5]

`--stretch` is the factor the logs were produced at; the margin is computed
against the budget the step would have at stretch 1, which is the harshest run
the gate admits.
"""

import glob
import io
import os
import re
import sys

HEAD = re.compile(r"^(THEOREM|LEMMA|PROPOSITION|COROLLARY)\s+(\w+)\s*==")
# <2>5.  <1>1a.  <3>.  and the bare <1> of a DEFINE or HIDE step
STEP = re.compile(r"^(\s*)<(\d+)>([\w]*)\s*\.?")
LOC = re.compile(r"@!!loc:(\d+):")
STATUS = re.compile(r"@!!status:(\w+)")
PROVER = re.compile(r"@!!prover:(\w+)")
METH = re.compile(r"@!!meth:([^\n]*)")
LIMIT = re.compile(r"time-limit: ([\d.]+)")
USED = re.compile(r"time-used: ([\d.]+)")
# What a BY may already name, so an annotated step is not reported again
BUDGET = re.compile(r"\b(SMTT|IsaT|ZenonT|Z3T|CVC4T|SpassT|ZipperT)\(\d+\)")


def addresses(path):
    """Maps each line of the module to its proof address."""
    lines = io.open(path, encoding="utf-8", errors="replace").read().split("\n")
    out = [""] * (len(lines) + 2)
    name = "<module>"
    stack = []          # (indent, label) innermost last
    anon = {}
    for i, line in enumerate(lines):
        h = HEAD.match(line)
        if h:
            name, stack, anon = h.group(2), [], {}
        else:
            s = STEP.match(line)
            if s:
                indent, level, label = len(s.group(1)), int(s.group(2)), s.group(3)
                if not label:
                    anon[level] = anon.get(level, 0) + 1
                    label = "_%d" % anon[level]
                while stack and stack[-1][0] >= level:
                    stack.pop()
                stack.append((level, label))
        path_str = ".".join(l for _, l in stack)
        out[i + 1] = "%s:%s" % (name, path_str) if path_str else name
    return out


def records(logs):
    """Yields (line, prover, limit, used) for every obligation a backend closed."""
    for f in logs:
        text = io.open(f, encoding="utf-8", errors="replace").read().replace("\0", "")
        for block in text.split("@!!BEGIN"):
            loc, st, pv, me = (LOC.search(block), STATUS.search(block),
                               PROVER.search(block), METH.search(block))
            if not (loc and st and pv and me) or st.group(1) != "proved":
                continue
            lim, use = LIMIT.search(me.group(1)), USED.search(me.group(1))
            if not (lim and use):
                continue
            yield int(loc.group(1)), pv.group(1), float(lim.group(1)), float(use.group(1))


def main(argv):
    args = [a for a in argv if not a.startswith("--")]
    opts = dict(a.lstrip("-").split("=", 1) for a in argv if "=" in a and a.startswith("--"))
    if len(args) < 2:
        print(__doc__)
        return 2
    module, logs = args[0], []
    for pattern in args[1:]:
        logs.extend(sorted(glob.glob(pattern)) or [pattern])
    margin = float(opts.get("margin", 5))
    stretch = float(opts.get("stretch", 5))

    addr = addresses(module)
    worst = {}
    for line, prover, limit, used in records(logs):
        a = addr[line] if line < len(addr) else "?"
        key = (a, prover)
        if key not in worst or used > worst[key][1]:
            worst[key] = (line, used, limit)

    src = io.open(module, encoding="utf-8", errors="replace").read().split("\n")
    flagged = []
    for (a, prover), (line, used, limit) in worst.items():
        at_one = limit / stretch
        if used * margin <= at_one:
            continue
        block = " ".join(src[line - 1:line + 7]).split(" DEF ")[0]
        flagged.append((a, prover, used, at_one, bool(BUDGET.search(block))))

    flagged.sort(key=lambda r: (-r[2], r[0]))
    todo = [r for r in flagged if not r[4]]
    print("%d obligations measured, %d under a %gx margin at stretch 1, "
          "%d of them not yet annotated"
          % (len(worst), len(flagged), margin, len(todo)))
    print()
    print("%-9s %6s %6s  %s" % ("prover", "used", "budget", "address"))
    for a, prover, used, at_one, annotated in flagged:
        print("%-9s %6.1f %6.0f  %s%s"
              % (prover, used, at_one, a, "" if not annotated else "   (annotated)"))
    return 1 if todo else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
