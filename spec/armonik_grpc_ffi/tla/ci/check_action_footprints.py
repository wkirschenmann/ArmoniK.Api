#!/usr/bin/env python3
"""Checks that every action of a specification mentions every variable.

An action that neither primes a variable nor lists it as UNCHANGED leaves it
free, so the action admits any successor value for it.  TLC reports that as
"Successor state is not completely specified", but only if the state is
reachable in a configuration someone runs; tlapm reports it as a proof that
mysteriously will not close, several hours later.  This check finds it in a
second, and it is the reason to keep it: adding a variable to the state means
touching every action, and forgetting one is silent.

A variable counts as mentioned when the action primes it, names it in an
UNCHANGED, or names a tuple definition that contains it.

Exit status 0 when every action is complete, 1 otherwise.
"""

import io
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
TLA = os.path.dirname(HERE)

# module -> (variables it must cover, tuple definitions, definitions that are
# not actions and so are exempt)
SPECS = [
    ("FfiGrpc.tla", "FfiGrpcState.tla"),
    ("AbstractGrpc.tla", "AbstractGrpcState.tla"),
]

# Definitions that build a state predicate rather than a step.  Init assigns
# every variable without priming, and the helpers below are fragments meant to
# be conjoined into an action, not actions themselves.
NOT_ACTIONS = {"Init", "TypeOK", "RequestCancellationOfActiveCalls",
               "HandPayloadToHost"}

# Temporal formulas are not actions: they quantify over behaviours.
# Fragments meant to be conjoined into an action.  They carry primes of their
# own, so an action that names one is covered for those variables - not
# following them reported seven complete actions as incomplete.
FRAGMENTS = {"RequestCancellationOfActiveCalls", "HandPayloadToHost"}

TEMPORAL = ("~>", "[]", "<>", "WF_", "SF_")


def strip_comments(text):
    """Comments are noise here, and worse: an apostrophe in English prose
    reads as a prime and makes an action look complete."""
    text = re.sub(r"\(\*.*?\*\)", " ", text, flags=re.S)
    return re.sub(r"\\\*.*$", "", text, flags=re.M)


def read(name):
    return strip_comments(io.open(os.path.join(TLA, name), encoding="utf-8").read())


def declared_variables(state_module):
    """The VARIABLES of a state module, plus those of the module it extends."""
    text = read(state_module)
    names = set()
    for m in re.finditer(r"^VARIABLES?\n((?:\s+\w+,?[^\n]*\n)+)", text, re.M):
        for line in m.group(1).splitlines():
            v = re.match(r"\s+(\w+)", line)
            if v:
                names.add(v.group(1))
    ext = re.search(r"^EXTENDS ([^\n]+)", text, re.M)
    if ext:
        for mod in (x.strip() for x in ext.group(1).split(",")):
            if mod.endswith("State") and os.path.exists(
                    os.path.join(TLA, mod + ".tla")):
                names |= declared_variables(mod + ".tla")
    return names


def tuple_definitions(text, variables):
    """name -> set of variables, for definitions of the form  Name == <<...>>."""
    out = {}
    # A tuple reached through an INSTANCE - l0_vars == L0!vars - covers the
    # whole shared state, which is declared in the level-0 state module.
    shared = declared_variables("AbstractGrpcState.tla")
    for m in re.finditer(r"^(\w+)\s*==\s*(\w+!vars)\s*$", text, re.M):
        out[m.group(1)] = set(shared)
    for m in re.finditer(r"^(\w+)\s*==\s*<<((?:[^>]|>(?!>))*)>>", text, re.M):
        body = m.group(2)
        out[m.group(1)] = {w for w in re.findall(r"\w+", body)}
    # A tuple may name another tuple; close over that.
    for _ in range(4):
        for k, v in out.items():
            for w in list(v):
                if w in out:
                    v |= out[w]
    return {k: (v & variables) | {w for w in v if w in out} for k, v in out.items()}


def actions(text):
    """name -> body, for every definition that describes a step.

    A refining action may prime nothing itself: it conjoins an L0 action and
    an UNCHANGED list, and every prime lives in what it names.  Requiring a
    prime in the body therefore skipped exactly the actions most likely to
    forget a variable - it hid RuntimeBeginShutdown, and with it every action
    of that shape."""
    parts = re.split(r"(?m)^(?=[A-Za-z]\w*(?:\([^)]*\))? ==)", text)
    out = {}
    for blk in parts:
        m = re.match(r"([A-Za-z]\w*)(?:\([^)]*\))? ==", blk)
        if not m or any(k in blk for k in TEMPORAL):
            continue
        # A prime or an UNCHANGED, and nothing else: naming an L0 operator is
        # not enough, or every state predicate that mentions L0!ChannelsOf
        # would be audited as a step.
        if "'" in blk or "UNCHANGED" in blk:
            out[m.group(1)] = blk
    return out


def definitions(text):
    """name -> body, split at every top-level definition."""
    parts = re.split(r"(?m)^(?=[A-Za-z]\w*(?:\([^)]*\))? ==)", text)
    out = {}
    for blk in parts:
        m = re.match(r"([A-Za-z]\w*)(?:\([^)]*\))? ==", blk)
        if m:
            out[m.group(1)] = blk
    return out


def fragment_writes(text, variables):
    """fragment -> the variables it accounts for, primed or left UNCHANGED."""
    out = {}
    for name, blk in definitions(text).items():
        if name not in FRAGMENTS:
            continue
        covered = {v for v in variables if re.search(r"\b" + v + r"'", blk)}
        for m in re.finditer(r"UNCHANGED\s+(<<((?:[^>]|>(?!>))*)>>|\w+)", blk):
            covered |= {w for w in re.findall(r"\w+", m.group(1))
                        if w in variables}
        out[name] = covered
    return out


def main():
    ok = True
    for spec, state in SPECS:
        text = read(spec)
        variables = declared_variables(state)
        tuples = tuple_definitions(text, variables)
        fragments = fragment_writes(text, variables)
        for name, body in sorted(actions(text).items()):
            if name in NOT_ACTIONS:
                continue
            mentioned = set()
            # A refining action conjoins an L0 action, and the level-0
            # machinery is the only legal writer of the level-0 state, so
            # naming it covers those variables.
            # An L0 action may take no arguments, so the parenthesis is not
            # part of the pattern: requiring it hid L0!RemainReleased.
            if re.search(r"\bL0![A-Z]\w*", body):
                mentioned |= declared_variables("AbstractGrpcState.tla")
            for frag, written in fragments.items():
                if re.search(r"\b" + frag + r"\b", body):
                    mentioned |= written
            for v in variables:
                if re.search(r"\b" + v + r"'", body):
                    mentioned.add(v)
            for m in re.finditer(r"UNCHANGED\s+(<<((?:[^>]|>(?!>))*)>>|\w+)", body):
                for w in re.findall(r"\w+", m.group(1)):
                    if w in variables:
                        mentioned.add(w)
                    for t, vs in tuples.items():
                        if w == t:
                            mentioned |= {x for x in vs if x in variables}
            missing = variables - mentioned
            if missing:
                ok = False
                print("  %s!%s leaves free: %s"
                      % (spec, name, ", ".join(sorted(missing))))
    if ok:
        print("OK: every action mentions every variable.")
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
