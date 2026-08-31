#!/usr/bin/env python3
"""Checks that every string literal a state variable is compared against, or
assigned, is one that variable can actually hold.

A retired value does not fail to parse and does not fail to type: in a
comparison it is simply always false, which makes a guard dead and a model
constraint prune more of the state space than intended - and the run still
reports every property clean.  That is the worst shape a defect can take
here, because nothing turns red.  A constraint reading
call_dispose_state = "disposed" after that value was renamed to "settled"
shrank two configurations exactly that way.

Assignments are covered too, and by choice rather than for symmetry: TypeOK
catches a bad one only in a run that reaches that branch, so an assignment
on a rare path can sit wrong indefinitely.

The binding comes from the typing conjuncts rather than from a table, so a
renamed state is caught wherever it is still spelled:

    /\\ var \\in [Dom -> SomeStates]        with SomeStates == {"a", "b"}
    /\\ var \\in OtherIds \\union {"none"}   admits the sentinel and nothing else

Only variables whose type resolves to literal sets this way are checked.
One typed over BOOLEAN, over a bare constant, or over an expression this
does not understand is skipped rather than guessed at, and the count in the
final line says how many are covered so that silence is never mistaken for
coverage.

Exit status 0 when every literal is admissible, 1 otherwise.
"""

import io
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TLA = os.path.dirname(HERE)

# A set definition whose right-hand side is entirely string literals, possibly
# spread over several lines: Name == {"a", "b",
#                                     "c"}
SET_DEF = re.compile(
    r'^([A-Za-z][A-Za-z0-9_]*)\s*==\s*\{([^}]*)\}', re.MULTILINE | re.DOTALL)

# A typing conjunct naming a set: var \in SetName, var \in [Dom -> SetName],
# var \in [A -> [B -> SetName]].  The set name is the last identifier.
TYPING = re.compile(
    r'/\\\s*([a-z][A-Za-z0-9_]*)\s+\\in\s+'
    r'(?:\[[^\]]*->\s*)*([A-Za-z][A-Za-z0-9_]*)\s*\]*\s*$', re.MULTILINE)

# A typing conjunct ending in an explicit literal set - the sentinel idiom,
# var \in OtherIds \union {"none"}.  Those literals are the only ones such a
# variable may be compared against, which is what makes a renamed sentinel
# visible here.
TYPING_UNION = re.compile(
    r'/\\\s*([a-z][A-Za-z0-9_]*)\s+\\in\s+[^\n]*?\\union\s*\{([^}]*)\}',
    re.MULTILINE)

LITERAL_ONLY = re.compile(r'^\s*(?:"[^"]*"\s*,?\s*)+$', re.DOTALL)
LITERAL = re.compile(r'"([^"]*)"')

# TLC writes counterexample modules beside the sources; they are untracked
# debris and must not decide whether this passes.
GENERATED = re.compile(r'_TTrace_\d+\.tla$')


def strip_comments(text):
    """Drops comments, so prose naming a retired value is not mistaken for
    code.  Block comments nest in TLA+, so the scan is by depth rather than
    by a non-greedy match, which would end the outermost comment at the
    first close and leave the remainder looking like code."""
    out, depth, i, n = [], 0, 0, len(text)
    while i < n:
        if text.startswith("(*", i):
            depth += 1
            i += 2
        elif depth and text.startswith("*)", i):
            depth -= 1
            i += 2
        elif depth:
            # Keep newlines so reported line numbers stay the file's own.
            out.append("\n" if text[i] == "\n" else " ")
            i += 1
        else:
            out.append(text[i])
            i += 1
    return re.sub(r'\\\*.*?$', '', "".join(out), flags=re.MULTILINE)


def balanced(text, start):
    """The bracketed expression beginning at start, brackets balanced, so a
    multi-line EXCEPT or function constructor is read whole.  An expression
    that does not close returns nothing rather than the rest of the file:
    sweeping to the end would attribute every later literal to this
    variable and fail the build on someone else's line, which is worse than
    checking one expression less."""
    depth, i, n = 0, start, len(text)
    while i < n:
        if text[i] == "[":
            depth += 1
        elif text[i] == "]":
            depth -= 1
            if depth == 0:
                return text[start:i + 1]
        i += 1
    return ""


def line_of(text, index):
    return text.count("\n", 0, index) + 1


def main():
    modules = sorted(f for f in os.listdir(TLA)
                     if f.endswith(".tla") and not GENERATED.search(f))
    bodies = {m: strip_comments(io.open(os.path.join(TLA, m),
                                       encoding="utf-8").read())
              for m in modules}

    # Every set of string literals, by name, across all modules: EXTENDS and
    # INSTANCE both make a level's sets visible to the level above.
    sets = {}
    for body in bodies.values():
        for name, inner in SET_DEF.findall(body):
            if LITERAL_ONLY.match(inner):
                sets[name] = set(LITERAL.findall(inner))

    var_sets = {}
    for body in bodies.values():
        for var, setname in TYPING.findall(body):
            if setname in sets:
                var_sets.setdefault(var, set()).update(sets[setname])
        for var, inner in TYPING_UNION.findall(body):
            if LITERAL_ONLY.match(inner):
                var_sets.setdefault(var, set()).update(LITERAL.findall(inner))

    if not var_sets:
        print("check_state_literals: no typed state variable found")
        return 1

    names = "|".join(sorted(var_sets, key=len, reverse=True))
    # var = "lit" and var # "lit", with any number of subscripts.
    cmp_re = re.compile(r'\b(' + names + r')\b(?:\[[^\]]*\])*\s*'
                        r"(?:'\s*)?(?:=|#)\s*\"([^\"]*)\"")
    # var \in {"a", "b"}
    in_re = re.compile(r'\b(' + names + r')\b(?:\[[^\]]*\])*\s*'
                       r"(?:'\s*)?\\in\s*\{([^}]*)\}")
    # An update or a function constructor: every literal inside the
    # bracketed expression must be one the variable can hold.
    write_re = re.compile(r'\[\s*(' + names + r')\s+EXCEPT\b')
    build_re = re.compile(r'\b(' + names + r")\b\s*'?\s*=\s*(\[)")

    # Every literal is counted at the position of its text in the file, so
    # a literal that a comparison and an enclosing update both reach is one
    # literal.  Every pass below records the offset of the content inside
    # the quotes, never the quote itself: mixing the two would put the same
    # literal at two positions and inflate the tally by exactly one each
    # time the passes overlap.
    counted, bad = set(), []
    for module, body in bodies.items():
        for m in cmp_re.finditer(body):
            counted.add((module, m.start(2)))
            if m.group(2) not in var_sets[m.group(1)]:
                # The literal's own offset, not the match's: the variable
                # and its set can sit on different lines, and the line
                # worth reporting is the one the bad value is written on.
                bad.append((module, line_of(body, m.start(2)),
                            m.group(1), m.group(2)))
        for m in in_re.finditer(body):
            if not LITERAL_ONLY.match(m.group(2)):
                continue
            for lit in LITERAL.finditer(m.group(2)):
                counted.add((module, m.start(2) + lit.start(1)))
                if lit.group(1) not in var_sets[m.group(1)]:
                    bad.append((module,
                                line_of(body, m.start(2) + lit.start(1)),
                                m.group(1), lit.group(1)))
        # An update matches both patterns at the same bracket - `var' =
        # [var EXCEPT ...]` is an assignment and a constructor at once - so
        # a region is taken once, or the literal count would claim more
        # coverage than there is.  The assigned variable wins the tie: in
        # `bar' = [foo EXCEPT ...]` the literals become bar's value, so
        # bar's set is the one that admits them, and attributing them to
        # foo would fail correct code.
        regions = {}
        for m in write_re.finditer(body):
            regions[m.start()] = m.group(1)
        for m in build_re.finditer(body):
            regions[m.start(2)] = m.group(1)
        for start, var in sorted(regions.items()):
            region = balanced(body, start)
            # A guard inside the region may compare a DIFFERENT variable
            # against its own value; those literals are that variable's,
            # and the comparison pass above already judged them there.
            # Harvesting them here would attribute them to this variable
            # and fail correct code.
            others = re.compile(r'\b(?!' + var + r'\b)(?:' + names + r')\b'
                                r'(?:\[[^\]]*\])*\s*(?:=|#)\s*"[^"]*"')
            # Blanked to the same length, so an offset in the masked region
            # is still an offset in the file and the line reported is the
            # literal's own rather than the construct's.
            masked = others.sub(lambda m: " " * len(m.group(0)), region)
            for m in LITERAL.finditer(masked):
                counted.add((module, start + m.start(1)))
                if m.group(1) not in var_sets[var]:
                    bad.append((module,
                                line_of(body, start + m.start()), var,
                                m.group(1)))

    if bad:
        print("Literals a state variable can never hold:")
        for module, lineno, var, lit in sorted(set(bad)):
            allowed = ", ".join(sorted(var_sets[var]))
            print('  %s:%d: %s = "%s" - it holds one of {%s}'
                  % (module, lineno, var, lit, allowed))
        return 1

    print("OK: %d typed state variables, %d literals, all admissible."
          % (len(var_sets), len(counted)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
