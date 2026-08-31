"""Splits the Passthrough case of Next, one step per level-1 action.

gen_case splits Next by disjunct; this splits the disjunct that carries the
whole level-1 machine.  Eighteen level-1 actions in one step time out where
each on its own is immediate - the same measurement that split Next, one
level down - and a goal reading a level-1 variable needs the level-0 action
behind the level-1 one, because a level-1 action states its level-0 frame
inside the level-0 action rather than in its own UNCHANGED.

The groups are read off RuntimeSteps and BindingDowncalls rather than
restated here, so a level-1 action added to a passthrough cannot be missed.

Run with sys.dont_write_bytecode, or clear __pycache__ after editing.
"""
import io
import re

from gen_case import D, FRAME, body_of, wrap

# A group binds the same identifier the lemma takes as a parameter, and
# shadowing it silently retargets the goal at the bound one.
RENAME = {"cId": "c2", "chId": "ch2", "rtId": "r2", "msg": "m2", "b": "b2"}


def pass_groups():
    """One entry per top-level disjunct of the level-1 half of Next:
    (binders, calls), each call already renamed."""
    src = io.open(D + "DotNetBinding.tla", encoding="utf-8").read()
    groups = []
    for name in ("RuntimeSteps", "BindingDowncalls"):
        body = body_of(name, src)
        chunks, cur = [], None
        for line in (x for x in body.split("\n") if x.strip()):
            if re.match(r"^    \\/ ", line):
                if cur:
                    chunks.append(cur)
                cur = [line]
            elif cur is not None:
                cur.append(line)
            else:
                cur = [line]
        if cur:
            chunks.append(cur)
        for chunk in chunks:
            text = "\n".join(chunk)
            binders = []
            m = re.search(r"\\E ([^:]+):", text)
            if m:
                for part in m.group(1).split(","):
                    v, dom = part.split("\\in")
                    binders.append((RENAME.get(v.strip(), v.strip()),
                                    dom.strip()))
            calls = []
            for act, args in re.findall(
                    r"(L1!(?:L0!)?[A-Z][A-Za-z0-9_]*)\(([^)]*)\)", text):
                inner = ", ".join(RENAME.get(x.strip(), x.strip())
                                  for x in args.split(","))
                calls.append("%s(%s)" % (act, inner))
            calls += re.findall(r"\\/ (L1!(?:L0!)?[A-Z][A-Za-z0-9_]*)\s*$",
                                text, re.M)
            if calls:
                groups.append((binders, calls))
    return groups


def behind(call):
    """The level-0 and level-1 ACTIONS a level-1 action's body composes,
    plus both frames.

    Actions only: a guard names no primed variable, so it cannot decide a
    frame goal, and carrying the guards is what made the three cases where
    the goal's variable actually moves time out.
    """
    l1 = (io.open(D + "FfiGrpc.tla", encoding="utf-8").read()
          + io.open(D + "FfiGrpcState.tla", encoding="utf-8").read())
    l0 = (io.open(D + "AbstractGrpc.tla", encoding="utf-8").read()
          + io.open(D + "AbstractGrpcState.tla", encoding="utf-8").read())
    src = {"L1!": l1, "L1!L0!": l0}
    verdict, out = {}, []

    def bare(name, prefix):
        """A definition's body without its comments.  body_of stops at the
        next definition, which sweeps in that definition's leading comment,
        and an apostrophe in prose reads as a prime: without this, a guard
        whose neighbour's comment says "the host's step" is taken for an
        action."""
        body = body_of(name, src[prefix])
        body = re.sub(r"\(\*.*?\*\)", "", body, flags=re.S)
        return re.sub(r"\\\*[^\n]*", "", body)

    def refs(prefix, body):
        """Every definition of either level the body composes."""
        found = [("L1!L0!", h) for h in
                 re.findall(r"\bL0!([A-Z][A-Za-z0-9_]*)", body)]
        for h in re.findall(r"\b([A-Z][A-Za-z0-9_]*)\(", body):
            if re.search(r"^%s\(" % h, src[prefix], re.M):
                found.append((prefix, h))
        return sorted(set(found))

    def is_action(prefix, name):
        """A name states a frame or a prime, here or under it.  A guard
        does neither, and carrying the guards is what made the three cases
        where the goal's variable actually moves time out."""
        if (prefix, name) in verdict:
            return verdict[(prefix, name)]
        verdict[(prefix, name)] = False      # cut cycles
        body = bare(name, prefix)
        yes = ("'" in body or "UNCHANGED" in body
               or any(is_action(*r) for r in refs(prefix, body)))
        verdict[(prefix, name)] = yes
        return yes

    def walk(prefix, name):
        for p, h in refs(prefix, bare(name, prefix)):
            if is_action(p, h) and p + h not in out:
                out.append(p + h)
                walk(p, h)

    walk("L1!", call.split("(")[0].split("!")[-1])
    return out + ["L1!vars", "L1!ffi_vars", "L1!l0_vars", "L1!L0!vars",
                  "L1!L0!RuntimeVars", "L1!L0!ChannelVars",
                  "L1!L0!CallVars"]


def build(name, assume, prove, goal_defs, method="SMT", deep=True):
    """deep: mine the level-0 action behind each level-1 one.  A goal about
    a managed variable does not need it - ManagedStutter frames the lot."""
    out = ["LEMMA %s ==" % name]
    out += ["    ASSUME " + assume[0]] + ["           " + a for a in assume[1:]]
    out.append("    PROVE  " + prove)
    out.append("<1>0. ManagedStutter")
    out.append("    BY DEF Passthrough")
    labels = []
    for k, (binders, calls) in enumerate(pass_groups(), start=1):
        lab = "<1>%d" % k
        labels.append(lab)
        head = ("\\E " + ", ".join("%s \\in %s" % b for b in binders) + " :"
                if binders else "")
        if len(calls) == 1:
            out.append("%s. CASE %s %s" % (lab, head, calls[0])
                       if head else "%s. CASE %s" % (lab, calls[0]))
        else:
            out.append("%s. CASE %s" % (lab, head))
            for i, c in enumerate(calls):
                out.append("          " + ("\\/ " if i else "   ") + c)
        def defs(c):
            ordered, seen = [], set()
            for d in ([c.split("(")[0]]
                      + ([] if not deep else behind(c)) + goal_defs + FRAME):
                if d not in seen:
                    seen.add(d)
                    ordered.append(d)
            return ordered
        if binders:
            out.append("  <2>0. SUFFICES ASSUME "
                       + ", ".join("NEW %s \\in %s" % b for b in binders)
                       + ",")
            for i, c in enumerate(calls):
                out.append("                        "
                           + ("\\/ " if i else "   ") + c)
            out.append("                 PROVE  "
                       + prove.replace("\n           ",
                                       "\n                        "))
            out.append("      BY " + lab)
            subs = []
            for i, c in enumerate(calls, start=1):
                subs.append("<2>%d" % i)
                out.append("  <2>%d. CASE %s" % (i, c))
                out += wrap("      BY <1>0, <2>0, <2>%d, %s DEF "
                            % (i, method), defs(c), cont="         ")
            out += wrap("  <2>%d. QED BY <2>0, " % (len(calls) + 1), subs,
                        cont="         ")
        else:
            out += wrap("    BY <1>0, %s, %s DEF " % (lab, method),
                        defs(calls[0]))
    out.append("<1>q. QED")
    out += wrap("    BY " + ", ".join(labels) + " DEF ",
                ["Passthrough", "RuntimeSteps", "BindingDowncalls"])
    return "\n".join(out)
