"""Builds a case analysis over Next, one step per top-level disjunct.

Why this exists: a single step over the whole of Next times out.  Forty
actions, each with a fourteen-variable frame, and usually a primed
hypothesis to propagate through all of them.  Split by disjunct and each
step carries only the actions it can see, which is what keeps the
definition list short enough for SMT.

What it knows, all of it learned by measurement:
- the disjunct carrying twenty actions splits again, one step per action,
  and the binder is renamed because the disjunct binds the same identifier
  the lemma takes as a parameter;
- the domain follows the binder - a channel disjunct binds a channel;
- an action built on another needs that one in its DEF list too;
- a goal reading a level-1 variable needs the level-1 action every level-2
  action rides on, in EVERY case, because a coupled action states its
  level-1 frame inside the level-1 action rather than in its own UNCHANGED;
- the facts of a SUFFICES are not ambient, so the disjunction elimination
  cites the SUFFICES step.

Usage: import build(name, assume, prove, goal_defs, deep, method, mine_l1)
and write what it returns into a proofs module.  goal_defs is the list the
GOAL reads - not the module's vocabulary; a goal about a managed variable
wants ManagedStutter and no more.

Run with sys.dont_write_bytecode, or clear __pycache__ after editing: a
stale bytecode file serves the old generator and looks exactly like a fix
that did not work.
"""
import io
import re

D = "C:/Users/wkirschenmann/Source/ArmoniK.Api/spec/armonik_grpc_ffi/tla/"
S = ("C:/Users/WKIRSC~1/AppData/Local/Temp/claude/"
     "C--Users-wkirschenmann-Source-ArmoniK-Api/"
     "dc182aea-658c-4075-b8e9-bc53eca55611/scratchpad/")

FRAME = ["ManagedStutter", "vars", "l1_vars", "managed_vars",
         "ManagedRuntimeVars", "ManagedChannelVars", "ManagedCallVars",
         "ReaderVars", "WriterVars"]


def body_of(name, text):
    pat = r'^%s(\([^)]*\))?\s*==(.*?)(?=\n[A-Za-z(]|\Z)' % name
    m = re.search(pat, text, re.S | re.M)
    return m.group(2) if m else ""


def next_parts():
    lines = io.open(D + "DotNetBinding.tla", encoding="utf-8").read().split("\n")
    i = lines.index("Next ==")
    j = i + 1
    while not lines[j].startswith("(***"):
        j += 1
    parts, cur = [], None
    for l in (x for x in lines[i + 1:j] if x.strip()):
        if l.startswith("    \\/ "):
            if cur:
                parts.append(cur)
            cur = [l]
        else:
            cur.append(l)
    parts.append(cur)
    return parts


def actions_of(part):
    text = "\n".join(part)
    named = set(re.findall(r'\b([A-Z][A-Za-z0-9_]*)\(', text))
    solo = set(re.findall(r'\\/ ([A-Z][A-Za-z0-9_]*)\s*$', text, re.M))
    solo |= set(re.findall(r'^    \\/ ([A-Z][A-Za-z0-9_]*)\s*$', text, re.M))
    body = io.open(D + "DotNetBinding.tla", encoding="utf-8").read()
    return sorted({n for n in named | solo
                   if re.search(r'^%s(\(|\s*==)' % n, body, re.M)})


def case_text(part):
    """The CASE expression: the disjunct without its leading \\/ ."""
    text = "\n".join(part)
    text = re.sub(r'^    \\/ ', "", text, count=1)
    return "\n".join(l[4:] if l.startswith("    ") else l
                     for l in text.split("\n"))


def wrap(prefix, items, width=74, cont="       "):
    lines, line = [], prefix
    for i, d in enumerate(items):
        piece = d + ("," if i < len(items) - 1 else "")
        if len(line) + 1 + len(piece) > width and line.strip():
            lines.append(line.rstrip())
            line = cont
        line += (" " if line.strip() else "") + piece
    lines.append(line.rstrip())
    return lines


def l1_behind(actions):
    """The level-1 actions those level-2 actions ride on, and the level-0
    ones behind them.  A goal about a level-1 variable needs them in EVERY
    case: a coupled action states its level-1 frame inside the level-1
    action, not in its own UNCHANGED."""
    l2 = io.open(D + "DotNetBinding.tla", encoding="utf-8").read()
    l1 = (io.open(D + "FfiGrpc.tla", encoding="utf-8").read()
          + io.open(D + "FfiGrpcState.tla", encoding="utf-8").read())
    out = []
    for a in actions:
        b = _body(a, l2)
        for g in sorted(set(re.findall(r'\bL1!([A-Z][A-Za-z0-9_]*)', b))):
            if g == "L0":
                continue
            out.append("L1!" + g)
            gb = _body(g, l1)
            for h in sorted(set(re.findall(r'\bL0!([A-Z][A-Za-z0-9_]*)', gb))):
                out.append("L1!L0!" + h)
        for g in sorted(set(re.findall(r'\bL1!L0!([A-Z][A-Za-z0-9_]*)', b))):
            out.append("L1!L0!" + g)
    return out


def _body(n, t):
    return body_of(n, t)


def build(name, assume, prove, goal_defs, deep=(), method="SMT",
          mine_l1=False):
    """deep: extra definitions the Passthrough case needs, for a goal that
    reads a level-1 or level-0 variable rather than a managed one."""
    parts = next_parts()
    out = ["LEMMA %s ==" % name]
    out += ["    ASSUME " + assume[0]] + ["           " + a for a in assume[1:]]
    out.append("    PROVE  " + prove)
    out.append("<1>0. CASE UNCHANGED vars")
    out += wrap("    BY <1>0, %s DEF " % method, goal_defs + FRAME)
    labels = ["<1>0"]
    for k, part in enumerate(parts, start=1):
        lab = "<1>%d" % k
        labels.append(lab)
        expr = case_text(part)
        head, rest = expr.split("\n", 1) if "\n" in expr else (expr, None)
        out.append("%s. CASE %s" % (lab, head))
        if rest:
            out += ["      " + l for l in rest.split("\n")]
        acts = actions_of(part)

        def deflist(subset):
            # an action built on another needs that one too:
            # DisposeCallForChannel is BeginDisposeCall under a guard
            l2src = io.open(D + "DotNetBinding.tla", encoding="utf-8").read()
            inner = []
            for a in subset:
                for n in re.findall(r'\b([A-Z][A-Za-z0-9_]*)\(',
                                    body_of(a, l2src)):
                    if n not in subset and re.search(
                            r'^%s\(' % n, l2src, re.M):
                        inner.append(n)
            whole = list(subset) + sorted(set(inner))
            ds = list(whole)
            if mine_l1:
                # mine the inner actions too: the level-1 action a composed
                # action rides on sits in the inner one's body
                ds += l1_behind(whole)
            if "Passthrough" in subset:
                ds += list(deep)
            ds += goal_defs + FRAME
            seen, ordered = set(), []
            for d in ds:
                if d not in seen:
                    seen.add(d)
                    ordered.append(d)
            return ordered

        # One disjunct of Next carries twenty actions, and a single step over
        # all of them times out where each on its own is immediate.
        single = re.findall(r'\\/ ([A-Z][A-Za-z0-9_]*)\((\w+)\)\s*$',
                            "\n".join(part), re.M)
        if len(acts) > 4 and len(single) == len(acts) and single:
            # a fresh name: the disjunct binds the same identifier the lemma
            # takes as a parameter, and shadowing it silently retargets the
            # goal at the bound one
            raw = single[0][1]
            # the domain follows the binder, not the call ids: a channel
            # disjunct binds a channel and a runtime one a runtime
            dom = {"cId": "CallIds", "chId": "ChannelIds",
                   "rtId": "RuntimeIds"}.get(raw, "CallIds")
            binder = {"cId": "c2", "chId": "ch2", "rtId": "r2"}.get(raw, raw)
            out.append("  <2>0. SUFFICES ASSUME NEW %s \\in %s,"
                       % (binder, dom))
            for i, (n, _) in enumerate(single):
                pad = "                        "
                out.append(pad + ("\\/ " if i else "   ") + "%s(%s)"
                           % (n, binder))
            out.append("                 PROVE  "
                       + prove.replace("\n           ",
                                       "\n                        "))
            out.append("      BY " + lab)
            subs = []
            for i, (n, _) in enumerate(single, start=1):
                subs.append("<2>%d" % i)
                out.append("  <2>%d. CASE %s(%s)" % (i, n, binder))
                out += ["    " + l for l in
                        wrap("      BY <2>0, <2>%d, %s DEF " % (i, method),
                             deflist([n]))]
            # the SUFFICES' disjunction is a fact, and a SUFFICES' facts are
            # not ambient: the elimination has to cite the step
            out += wrap("  <2>%d. QED BY <2>0, " % (len(single) + 1), subs,
                        cont="         ")
            continue
        out += wrap("    BY %s, %s DEF " % (lab, method), deflist(acts))
    out.append("<1>q. QED")
    out.append("    BY " + ", ".join(labels) + " DEF Next")
    return "\n".join(out)
