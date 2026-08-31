"""Builds the temporal skeleton of a one-edge leads-to.

Every promise of this level has the same five pieces: the target state is
reached by one action, that action is enabled while the source state holds,
the source state holds until it fires, weak fairness turns the two into a
step, and the whole is lifted from Spec.  Only the three step lemmas carry
content; the two theorems above them are boilerplate, and getting their
shape wrong costs a full prover round each time.

What the shape has to respect, all of it learned by measurement:
- the necessitation is taken in a theorem whose temporal hypotheses are
  boxed, because PTL will not necessitate under an unboxed one and Spec is
  unboxed;
- a conjunction handed to the temporal prover is spelled infix, in the
  cited lemma's own order: a bulleted junction and an infix one are
  unrelated atoms to it;
- the until-step states the source state again, whole, beside the target:
  "the wait is over" and "the wait is not still on" are two atoms with
  nothing between them, so P' \\/ Q' is the only form that closes.

Usage: build(...) returns the two theorems.  The three step lemmas are
written by hand - their definition lists follow the goal, which is what no
generator can guess.
"""

TIERS = {"binding": "BindingOwedFairness",
         "runtime": "RuntimeOwedFairness",
         "application": "ApplicationOwedFairness"}


def _infix(parts, indent):
    """A conjunction the temporal prover reads as one: first part bare,
    the rest continued, never a leading bullet."""
    out = [parts[0]]
    for p in parts[1:]:
        out.append(" " * indent + "/\\ " + p)
    return out


def build(name, binder, domain, boxes, wf, source, target, prop,
          edge_lemmas, tier="binding", extra_boxes=()):
    """name: the property's own name, as declared.
    binder/domain: the quantifier the property carries, or ("", "").
    boxes: the boxed invariants the step lemmas ask for.
    wf: the action expression weak fairness is stated on.
    source/target: the two state predicates, as the property spells them.
    prop: the property's body under the binder, for the SUFFICES.
    edge_lemmas: (enabled, lands, holds) lemma names.
    """
    enabled, lands, holds = edge_lemmas
    par = "NEW %s \\in %s" % (binder, domain) if binder else ""
    arg = "(%s)" % binder if binder else ""
    boxed = list(boxes)
    hyp = ["[]" + b for b in boxed] + ["[][Next]_vars",
                                       "WF_vars(%s)" % wf]
    src = source
    tgt = target

    out = []
    out.append("\\* The edge, as weak fairness reads it: enabled while the")
    out.append("\\* source holds, landing on the target, and the source")
    out.append("\\* holding until it fires.  Stated with the boxes as")
    out.append("\\* hypotheses rather than under Spec, because a")
    out.append("\\* necessitation cannot be taken under an unboxed one.")
    out.append("THEOREM %sAlwaysEnds ==" % name)
    if par:
        out.append("    ASSUME %s" % par)
        out.append("    PROVE  /\\ " + hyp[0])
        pad = "           "
    else:
        out.append("    /\\ " + hyp[0])
        pad = "    "
    for h in hyp[1:]:
        out.append(pad + "/\\ " + h)
    out.append(pad + "/\\ " + src)
    out.append(pad + "=> <>(" + tgt + ")")
    line, wrapped = "<1>1. ASSUME", []
    for h in hyp + [src]:
        piece = h + ("," if h != src else "")
        if len(line) + 1 + len(piece) > 74 and line.strip():
            wrapped.append(line)
            line = "            "
        line += " " + piece
    wrapped.append(line)
    out += wrapped
    out.append("      PROVE  <>(" + tgt + ")")
    if "ManagedTypeOK" in boxed:
        # PTL, not the default cascade: a WF_ sits in the context, and SMT
        # calls it an unsupported expression
        out.append("  <2>1. []ManagedTypeOK BY <1>1, PTL")
    else:
        out.append("  <2>1. []ManagedTypeOK")
        out.append("    BY <1>1, PTL DEF ManagedIndInv")
    ante = _infix(boxed + [src], 11)
    out.append("  <2>2. [](" + ante[0])
    out += ["      " + a for a in ante[1:]]
    out.append("               => ENABLED <<%s>>_vars)" % wf)
    out.append("    BY %s, PTL" % enabled)
    out.append("  <2>3. [](ManagedTypeOK /\\ <<%s>>_vars" % wf)
    out.append("               => (" + tgt + ")')")
    out.append("    BY %s, PTL" % lands)
    ante2 = _infix(boxed + [src, "[Next]_vars"], 11)
    out.append("  <2>4. [](" + ante2[0])
    out += ["      " + a for a in ante2[1:]]
    out.append("               => \\/ (" + src + ")'")
    out.append("                  \\/ (" + tgt + ")')")
    out.append("    BY %s, PTL" % holds)
    out.append("  <2>5. QED BY <1>1, <2>1, <2>2, <2>3, <2>4, PTL")
    out.append("<1>2. QED BY <1>1, PTL")
    out.append("")
    out.append("THEOREM %sHolds == Spec => %s" % (name, name))
    if par:
        out.append("<1>0. SUFFICES ASSUME Spec, %s" % par)
        out.append("               PROVE  " + prop)
    else:
        out.append("<1>0. SUFFICES ASSUME Spec")
        out.append("               PROVE  " + prop)
    out.append("    BY DEF %s" % name)
    out.append("<1>1. /\\ " + "\n      /\\ ".join(
        ["[]" + b for b in boxed] + ["[][Next]_vars"]))
    out.append("    BY <1>0, ManagedIndInvHolds, ManagedTypeOKHolds,")
    out.append("       DerivedInvariantsHold, PTL DEF Spec")
    out.append("<1>2. WF_vars(%s)" % wf)
    out.append("    BY <1>0, Isa DEF Spec, Fairness, %s" % TIERS[tier])
    out.append("<1>3. QED BY <1>1, <1>2, %sAlwaysEnds, PTL" % name)
    return "\n".join(out)
