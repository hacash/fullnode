# Call-data materialization

`extract_call_data` turns the ACTION body / Concat-NTFUNC argv / calc input
slot into a raw byte body. `Nil` is `[]`. A one-layer `Compo` list of
Bytes-like scalars is deferred CAT: each element follows CAT's `extract_bytes`
rules (`Nil` rejected) and the chunks are concatenated. Total length is capped
by `SpaceCap::call_data_size` (`CALL_DATA_MAX`, 5120; the same ceiling as the
calc wire cap `CALC_IO_MAX`). Ordinary `Value::valid` / CAT / PUT / HREAD /
return values still use `value_size` (1280) only.

## Packed vs Concat

`Tuple` is the **packed argv container**: user calls with 2+ params, `Packed`
native funcs with 2+ args, and the explicit `tuple(...)` builtin all build one.
`Concat` is the **byte-string ABI**: the compiler `CAT`s its args into one
`Bytes`, and a raw-bytecode caller may instead pass a one-layer `Compo` list
(deferred CAT).

`extract_call_data` therefore rejects `Tuple`. This is a validation invariant,
not a missing case: it keeps the value shape from reinterpreting packed argv as
a byte string at the two most permissive boundaries (ACTION bodies and
Concat-NTFUNC), so a multi-arg `tuple(amount, address)` cannot silently turn
into an action body. A one-layer list already provides the same multi-chunk
bytes, so Tuple flattening would add ambiguity, not capability. The interpreter
selects `Concat`/`Packed` from `NativeFunc::argv_pack`; the materializer's
Tuple rejection is the last line of defense for raw bytecode that bypasses the
compiler.

## Calc input

Calc uses the same `extract_call_data` and the same ceiling. A kernel that
needs more than the 1,280 B HVM scalar cap spreads its input over a one-layer
`list { chunk, ... }` (the fitsh `list` literal packs a `Compo` list); each
chunk is still capped by `value_size`, and the flattened total by
`call_data_size`. A kernel's declared `input_max` (≤ `CALC_IO_MAX`, the same
5120) is enforced by `CalcFnObj::run`'s pre-flight, where over-input is
`CalcAbort2::Trap` with 0 cycles. `tuple(...)` is not a byte carrier at any
call boundary, calc included.
