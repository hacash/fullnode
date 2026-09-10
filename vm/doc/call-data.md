# Call-data materialization

`extract_call_data` turns the ACTION / Concat-NTFUNC argv slot into a raw byte
body. `Nil` is `[]`. A one-layer `Compo` list of Bytes-like scalars is deferred
CAT: each element follows CAT's `extract_bytes` rules (`Nil` rejected) and the
chunks are concatenated. Total length is capped by `SpaceCap::call_data_size`
(4608). Ordinary `Value::valid` / CAT / PUT / HREAD / return values still use
`value_size` (1280) only.

Concat list = byte fragments (deferred CAT). Packed list = argv vector. The
interpreter chooses by `NativeFunc::argv_pack`: Concat calls `extract_call_data`;
Packed never does.
