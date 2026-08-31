# Vendored TLA+ modules

`Folds.tla` and `Functions.tla` are copies of the modules shipped with TLAPS,
taken verbatim from the standard library of the prover this specification is
verified with (fork `qdelamea-aneo/tlapm`, `/root/tlapm-opt-wil/lib/tlaps`).

They are here for one reason: **tlapm resolves them, SANY does not.** `FfiGrpc`
extends `Functions` for `SumFunctionOnSet`, which carries the byte accounting;
without the copies, `ci/check.sh` cannot parse the modules that use it and the
SANY half of the gate reports nothing rather than failing loudly.

Do not edit them. The theorems cited from them - `SumFunctionOnSetAddIndex`,
`SumFunctionOnSetRemoveIndex`, `SumFunctionOnSetEqual`, `SumFunctionOnSetNat` -
live in `FunctionTheorems.tla`, which is not copied here because only tlapm
ever reads it.
