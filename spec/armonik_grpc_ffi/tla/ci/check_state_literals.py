#!/usr/bin/env python3
"""Checks that every string literal compared against a state variable is one
the variable can actually hold.

A literal that no longer belongs to its variable's state set does not fail to
parse and does not fail to type: the comparison is simply always false.  In a
guard it makes an action dead; in a model constraint it prunes more of the
state space than intended, and the run still reports every property clean.
That is the worst shape a defect can take here, because nothing turns red -
a constraint reading call_dispose_state = "disposed" after that value was
renamed to "settled" silently shrank two configurations.

The binding is taken from the typing conjuncts rather than from a table: a
line of the form

    /\\ var \\in [Dom -> SomeStates]

says what var may hold, and SomeStates == {"a", "b"} says what those are.
Only variables whose type resolves to such a literal set are checked; a
variable typed over BOOLEAN, over a constant, or over an expression this
does not understand is skipped rather than guessed at.

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

# A typing conjunct: var \in SetName, or var \in [Dom -> SetName], or
# var \in [A -> [B -> SetName]].  The set name is the last identifier.
TYPING = re.compile(
    r'/\\\s*([a-z][A-Za-z0-9_]*)\s+\\in\s+'
    r'(?:\[[^\]]*->\s*)*([A-Za-z][A-Za-z0-9_]*)\s*\]*\s*$', re.MULTILINE)

LITERAL_ONLY = re.compile(r'^\s*(?:"[^"]*"\s*,?\s*)+$', re.DOTALL)


def read(path):
    return io.open(path, encoding="utf-8").read()


def strip_comments(text):
    """Drops line comments and block comments, so prose naming a retired
    value is not mistaken for a comparison against it."""
    text = re.sub(r'\(\*.*?\*\)', '', text, flags=re.DOTALL)
    return re.sub(r'\\\*.*?$', '', text, flags=re.MULTILINE)


def main():
    modules = sorted(f for f in os.listdir(TLA) if f.endswith(".tla"))
    bodies = {m: strip_comments(read(os.path.join(TLA, m))) for m in modules}

    # Every set of string literals, by name, across all modules: EXTENDS and
    # INSTANCE both make a level's sets visible to the level above.
    sets = {}
    for body in bodies.values():
        for name, inner in SET_DEF.findall(body):
            if LITERAL_ONLY.match(inner):
                sets[name] = set(re.findall(r'"([^"]*)"', inner))

    # Every variable whose declared type is one of those sets.
    var_sets = {}
    for body in bodies.values():
        for var, setname in TYPING.findall(body):
            if setname in sets:
                var_sets.setdefault(var, set()).update(sets[setname])

    if not var_sets:
        print("check_state_literals: no typed state variable found")
        return 1

    names = "|".join(sorted(var_sets, key=len, reverse=True))
    # var = "lit" / var # "lit", with any number of subscripts, and the
    # set form var[...] \in {"a", "b"}.
    cmp_re = re.compile(
        r'\b(' + names + r')\b((?:\[[^\]]*\])*)\s*'
        r"(?:'\s*)?(?:=|#)\s*\"([^\"]*)\"")
    in_re = re.compile(
        r'\b(' + names + r')\b((?:\[[^\]]*\])*)\s*'
        r"(?:'\s*)?\\in\s*\{([^}]*)\}")

    bad = []
    for module, body in bodies.items():
        for lineno, line in enumerate(body.splitlines(), 1):
            for var, _subs, lit in cmp_re.findall(line):
                if lit not in var_sets[var]:
                    bad.append((module, lineno, var, lit))
            for var, _subs, inner in in_re.findall(line):
                if not LITERAL_ONLY.match(inner):
                    continue
                for lit in re.findall(r'"([^"]*)"', inner):
                    if lit not in var_sets[var]:
                        bad.append((module, lineno, var, lit))

    if bad:
        print("Literals a state variable can never hold:")
        for module, lineno, var, lit in bad:
            allowed = ", ".join(sorted(var_sets[var]))
            print('  %s:%d: %s = "%s" - it holds one of {%s}'
                  % (module, lineno, var, lit, allowed))
        return 1

    print("OK: %d typed state variables, every literal admissible."
          % len(var_sets))
    return 0


if __name__ == "__main__":
    sys.exit(main())
