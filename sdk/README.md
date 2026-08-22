# Hacash Unified SDK 2.0

The fullnode WASM SDK rebuilt under Unified SDK 2.0 (doc 14 `unified-sdk-major-version-design.md`). Design points:

- **No v1/v2 namespaces**: one surface, one release line; the exposed
  capability set is exactly the operation registry (`profile::OPERATIONS`),
  and versioning is carried by `system.sdk_version` (ABI major/minor).
- **Private keys never cross the SDK boundary**: the SDK only produces
  `SigningRequest` (digest + bindings) and consumes `SignatureProof`; the
  wallet vault performs signing.
- **JSON boundary + single engine**: raw WASM exposes only
  `sdk_invoke_json(operation_id, payload)` and `sdk_transport_version()`;
  requests/responses are JSON strings (envelope
  `{"ok":1,"body":...}` / `{"ok":0,"code":N,"msg":"...","detail":"..."}`),
  produced and consumed by the **hand-written JSON engine** (`field::json_*`)
  inside the wasm core — the codebase has exactly one JSON parser, and
  serde_json exists only as a Rust test oracle. Numeric fields travel as
  decimal strings so `JSON.parse` cannot lose precision. `tx.build` accepts
  wire-shaped actions (`kind` + `ActionSchema` field names); the JS facade
  only `JSON.stringify`s the request and `JSON.parse`s the envelope. It does
  not expose domain methods, rename fields, translate errors, or maintain a
  second operation registry.
  Adding an operation never changes the WASM export surface.
- **Transaction state machine**:
  `tx.build → inspect → prepare_signature → (vault) attach_signature → verify`;
  the review object `Review` is generated locally and binds `review_binding`.
- **Kind profile extension**: a new wallet action only extends the SDK
  codec catalog; there is no second friendly mapping table.

## Raw JS transport

```js
const sdk = await create_hacash_sdk({ target: "node" }); // auto|node|web
const envelope = sdk.sdk_invoke_json(2, { spec });
// envelope is the Rust JSON result, e.g. { ok: 1, body: ... }.
// Operation ids and payload fields are the WASM ABI; see `profile::OPERATIONS`.

```

`tx.prepare_signature`'s `opts.policy` is evaluated by the SDK itself: the
decision (including `deny`) is bound into the `SigningRequest` as a
`PolicyDecision`, so the caller cannot forge the outcome; whether a `deny`
stops signing is the upper layer's call, and the SDK never refuses
prepare/attach because of it. `tx.attach_signature` (full chain) requires
`review` + `request`: the request's id/binding is recomputed and verified
(editing any field — including `expires_at` — fails with
`invalid_signing_request`), and it checks digest/body_hash/signer/purpose/
algorithm, the proof↔request binding, the policy decision and the review
binding. Cold-signing paths without an approval chain use
`attach_signature_unbound` (only body/signature/limits are checked;
non-required signers are rejected only for type-3 — the chain's exact D-set
rule; type-2 tolerates extra signatures and the SDK attaches them too,
reporting completeness via `complete`/`missing_signers`). `tx.encode`
enforces that the rebuilt body's `unsigned_body_hash` matches the declared
value — tampering with an action's json fails with
`transaction_json_mismatch`; when a `review` is supplied its binding is
verified as well.

All input objects reject unknown fields (a typo reports
`unknown_field`/`unknown_action` instead of being silently ignored);
`system.sdk_version` and the dispatcher share the same
`OPERATIONS` registry (a test guarantees they do not drift). In a review,
`chain_ids_allowed` is the intersection of the `ChainAllow` actions (the
protocol executes each independently), and `valid_height_range` is likewise
an intersection; strict-mode `expired_height`/`wrong_chain` are derived facts
from the caller's context, and the SDK never withholds a review because of
them — whether to proceed is the upper layer's decision. The same applies to
`topology_violations`: the protocol action-tree analysis (scope / min tx
type / AST depth / top-rule) is reported as facts, and the SDK never refuses
inspect or build because of them.

Errors remain in the Rust envelope (`ok: 0`, numeric `code`, `msg`, `detail`).
The JS layer does not map numeric codes to friendly names.

## External interface (compiled `dist/` artifacts)

The shipped surface is deliberately small and stable: the WASM core exports
exactly two functions, the JS facade adds one loader, and every capability is
an operation id on that one transport. The operation registry lives only in
Rust (`profile::OPERATIONS`); the JS layer forwards `operation_id` + JSON
payload verbatim, so **adding an operation never changes the WASM/JS export
surface** — only the operation table below grows.

### 1. Transport surface

| Layer | Export | Signature | Description |
|---|---|---|---|
| WASM | `sdk_transport_version()` | `() -> number` | Transport version of the JSON envelope semantics (currently `9`). |
| WASM | `sdk_invoke_json(operation_id, payload)` | `(number, Uint8Array) -> string` | UTF-8 JSON request → JSON envelope string. `operation_id` is an `OP_*` constant (table below). |
| JS (node) | `create_hacash_sdk({ target: "auto"\|"node"\|"web", wasm? })` | `() -> Promise<{ sdk_invoke_json, sdk_transport_version }>` | Loads the matching backend: node auto-imports `../nodejs/`, web fetches `../web/` (optionally `{ wasm }` for a custom URL/Response). |
| JS (web) | `default __wbg_init(module_or_path?)`, `initSync(module)` | — | wasm-bindgen boilerplate for embedding; normal users go through `create_hacash_sdk`. |

All numeric request/response fields travel as **decimal strings** (JSON
`JSON.parse` never loses precision); hex strings may carry an optional `0x`
prefix. Private keys never cross the boundary — only public keys, digests,
signing requests, and signature proofs.

### 2. Result envelope

Every call returns one of:

```json
{ "ok": 1, "body": { ... } }                    // success
{ "ok": 0, "code": 3, "msg": "...", "detail": "..." } // error
```

`code` is the numeric id of a stable error code (`SdkErrorCode::ERROR_CODES`
order, `1` = `unknown_operation`, `0` = unknown code). `detail` is optional
JSON carrying `action_index`, `byte_offset`, `expected`/`actual`, etc.

### 3. Operations (`OP_*` registry, transport version 9)

| ID | Operation | Request fields | Response schema |
|---|---|---|---|
| 1 | `system.sdk_version` | — | `sdk-version@1`: `schema`, `package_version`, `abi{major,minor}` |
| 2 | `tx.build` | `spec` (TransactionSpec JSON: `tx_type` 2/3, `main`, `fee`, `timestamp?`, `gas_max?`, `actions[{kind, ...schema fields}]`) | `built-transaction@1`: `schema`, `tx_type`, `timestamp`, `main`, `fee`, `hash`, `hash_with_fee`, `unsigned_body_hash`, `body` (hex) |
| 3 | `tx.inspect_report` | `body` (hex), `signer_address?`, `describe?` | `review@5` (protocol facts; never a denial) |
| 4 | `tx.inspect` | `body`, `signer_address?`, `context{current_height, expected_chain_id, consensus_flags?}`, `describe?` | `review@5` with `expired_height`/`wrong_chain` facts bound in |
| 5 | `tx.prepare_signature` | `body`, `signer_address`, `options.review?`, `options.policy?`, `options.origin?`, `options.expires_at?` | `signing-request@1`: `id`, `purpose`, `algorithm`, `signer_address`, `digest`, `body_hash`, `review_binding?`, `policy_decision?`, `origin?`, `expires_at?`, `request_binding` |
| 6 | `tx.attach_signature` | `body`, `proof`, `review`, `request` | `attach-result@2`: `body`, `complete`, `present_signers`, `valid_signers`, `missing_signers`, `invalid_signers`, `signature_errors` |
| 7 | `tx.attach_signature_unbound` | `body`, `proof` | `attach-result@2` (no approval-chain checks) |
| 8 | `tx.verify` | `body` | `verify-result@1`: `ok`, `errors` |
| 9 | `tx.signature_report` | `body` | `signature-report@1`: `required`, `present`, `valid`, `missing`, `invalid` |
| 10 | `tx.decode` | `body`, `describe?` | `transaction-json@2`: `tx_type`, `timestamp`, `main`, `fee`, `gas_max`, `tx_hash`, `hash_with_fee`, `unsigned_body_hash`, `actions[]` (`action-desc@2`), `signatures[{public_key, signature}]` |
| 11 | `tx.encode` | `transaction` (a `tx.decode` output), `review?` | `built-transaction@1`; rebuilt `unsigned_body_hash` must match (else `transaction_json_mismatch`) |
| 12 | `account.verify_address` | `address` | `{ok, error?, address?}` (canonical readable form) |
| 13 | `account.address_from_public_key` | `public_key` (33-byte compressed hex) | `{address, version}` |
| 14 | `amount.parse` | `value` | `{value, unit, is_negative}` |
| 15 | `amount.format` | `value`, `unit` (u8) | `{value}` (exact decimal string) |
| 16 | `message.prepare_signature` | `params{digest, signer_address, origin?, expires_at?}` | `signing-request@1` with `purpose: "authentication"` |
| 17 | `message.verify` | `request`, `proof` | `{ok, address?, error?}` |
| 18 | `policy.evaluate` | `review`, `policy?` | `policy-decision@1`: `policy_id`, `policy_hash`, `review_binding`, `decision` (`allow`/`confirm`/`deny`), `findings`, `policy_binding` |
| 19 | `system.params` | — | `params@1`: `params_version`, `chain_id`, `ast_tree_depth_max`, `max_type3_signers`, `fee_purity_floor`, `fee_purity_reductions`, `max_tx_size`, `tx_actions_max`, `registered_tx_types`, `diamond_form_flag` |
| 20 | `tx.estimate_fee` | `body`, `height?` (defaults to the initial floor) | `fee-estimate@1`: `tx_type`, `height`, `fee_purity_floor`, `billing_size`, `minimum_fee?` (type-3 only), `fee`, `fee_purity`, `fee_enough` |
| 21 | `account.verify_signature` | `public_key` (33-byte hex), `digest` (32-byte hex), `signature` (64-byte hex) | `{ok, address?, error?}` (raw primitive; exchange API-signature checks) |
| 22 | `diamond.lookup` | exactly one of `name` / `serial` | `diamond-lookup@1`: `valid`, `name?`, `serial?`, `error?` |
| 23 | `vm.decode_call` | `action` (raw wire hex of a `contract_main_call`, e.g. `tx.decode`'s `actions[].raw`) | `vm-call@1`: `kind`, `name`, `scope`, `marks`, `marks_valid`, `codeconf`, `code_type`, `code_type_name`, `codes_len`, `codes_hash`, `codes_preview` |
| 24 | `action.describe` | `action` (raw wire hex of any action), `describe?` | `action-desc@2`: single-action description with independently switchable `description` / `json` / `code` facets |
| 25 | `vm.code` | `codes` (hex), `code_type` (0=bytecode, 1=ir_node), `format?` (`assembly` for bytecode; `fitsh`/`tree` for ir), `sourcemap?`, `limit?` (default 8000), `offset?` | `vm-code@1`: `code_type`, `code_type_name`, `codes_len`, `codes_hash`, `format`, `lines`, `text`, `truncated`, `limit`, `offset` |

Optional fields are exactly as written (`?`); unknown or duplicated request
fields are rejected with `unknown_field`/`parse_failed`. The shared signing
flow is `tx.build → tx.inspect → tx.prepare_signature → (vault signs the
digest) → tx.attach_signature → tx.verify`.

**`describe` facet control** (`tx.inspect_report` / `tx.inspect` / `tx.decode` /
`action.describe`): an optional `{description, json, code}` boolean object with
all facets on by default. `description` is the schema-declared one-line text,
`json` the canonical field-level JSON (large for contract deploy/update), and
`code` the VM code metadata of code-carrying actions (`contract_main_call`,
`p2sh`); turning facets off trims the review payload. `action-desc@2` adds
these facets to every action entry (`description?`/`json?`/`code?`, the latter
with `codeconf`/`code_type`/`code_type_name`/`codes_len`/`codes_hash`/
`codes_preview`).

**Code display** (`vm.decode_call` + `vm.code`): short maincall code can render
inline from `action-desc@2`'s `code` metadata + `codes_preview`; long code
opens a viewer that calls `vm.code` with `format`/`limit`/`offset` paging and
an optional external `sourcemap` (lib/function/slot/const names) for maximum
readability. Bytecode disassembles to annotated assembly; IR decompiles to
fitsh source (or a structural `tree` view). All decompilation is offline and
codec-only — no VM execution, no node.

### 4. Registered codec surface

The codec set is fixed by the SDK build and its selection rules; it is not
exposed as an operation (the full `CodecProfile` remains available to native
Rust callers via `sdk::profile::CodecProfile::standard()`).

- **Transaction envelopes:** Type 2 and Type 3 (Type 1 is deprecated and
  excluded); `tx.build`/`tx.decode` reject anything else.
- **Actions:** 35 wallet-reachable kinds across the `protocol` / `mint-core` /
  `vm` catalogs — every action whose scope is not `CALL_ONLY` (VM env/view
  syscalls such as `block_height`, `balance` are excluded because they can only
  run inside a contract).
- **Policy defaults:** absent `policy` fields take protocol defaults; the
  `deny_kinds` walk recurses into AST children, and `deny_blob` reads the
  schema-declared blob fact (`tx_message`, `tx_blob`).

### 5. dist layout

```
dist/
├── js/hacashsdk.mjs        # raw JSON facade (create_hacash_sdk), platform-agnostic
├── nodejs/hacashsdk.js     # wasm-bindgen node glue  + hacashsdk_bg.wasm (+ .d.ts)
├── web/hacashsdk.js        # wasm-bindgen web glue    + hacashsdk_bg.wasm (+ .d.ts)
└── page/hacashsdk_bg.js    # single-file browser build, wasm base64-inlined
```

## Building

Prerequisites (one-time): `rustup target add wasm32-unknown-unknown`,
`cargo install -f wasm-bindgen-cli --version 0.2.100` (the version is pinned;
build.sh validates it and prints install hints); `wasm-opt` and a JS
minifier (esbuild / terser, or esbuild fetched on demand via npx) are
optional and degrade gracefully when missing. `wasm-opt` needs explicit
`--enable-bulk-memory --enable-sign-ext` (newer rustc wasm output contains
these instructions; the old `--all-features` flags produce an invalid
module; build.sh handles this with a validation fallback).

```sh
./sdk/pack.sh             # regular build (readable JS)
./sdk/pack.sh --release   # minified JS (facade + glue + page assets)
```

One command builds the nodejs/web/no-modules wasm targets, assembles `dist/`,
and optionally minifies. All artifacts land in `sdk/dist/`:

- `js/hacashsdk.mjs` — raw JSON transport (node auto-loads `../nodejs/`;
  web uses `create_hacash_sdk({ wasm })` to load `../web/`).
- `nodejs/`, `web/` — the platform wasm-bindgen low-level glue + wasm.
- `page/` — a single-file browser build: `hacashsdk_bg.js` (base64-inlined
  wasm).

Build artifacts (`dist/`, `*.bak`, ...) are git-ignored (see `sdk/.gitignore`).

### Unused-import warnings in execute-off builds

The SDK compiles protocol/vm/mint-core with `default-features = false`
(`execute` off). `ActionRef`/`TxRef` stay the wire view in every build;
execute is a separate trait reached via `as_execute`. protocol's `exec/`
and vm's `api`/`fitshc`, `action/*_exec.rs`, and the IR/machine engine
(`ir`/`interpreter`/`frame`/`native`/`space`/`state`/`setup`) are switched
off by module. base's `chain` / `store` / `ledger` / `node` / `api` /
`sync` / `scaner`, `iface::{context,store}`, `state::chunk`, and
`ActionExecute` / `TransactionExecute` / `ExecRegistry` / `Vm` are likewise
off; the codec side keeps the `iface` wire traits, `StateRead` / keys, the
`registry` wire half, `runtime` scope/gas, and the `action` / `contract` /
`rt` wire types / `value::ContractAddress`. `check-wasm-graph.sh` guards
that the wasm graph contains no `execute` / `serde_json` / `dyn-clone` /
`blake2` / `tiny-keccak` / `x16rs` / `num-bigint`;
`check-architecture.sh` further guards that the production dependency
graphs of field/sys/base/mint/app/vm contain no serde_json (serde is
allowed only transitively via libsecp256k1 / axum / reqwest /
serde_urlencoded).

## Native Rust usage

The rlib is equally usable: `sdk::inspect::inspect_report`,
`sdk::attach::*`, `build_transaction`, `evaluate_policy` and the other
strongly-typed APIs share the same protocol logic and vectors as the WASM
build.

## Testing

```sh
cargo test -p sdk          # unit/flow tests (wire-spec build, signature flows, guard/topology facts)
node ./sdk/tests/...       # packaged-JS smoke tests (after ./sdk/pack.sh)
```

## Status (M1)

- Implemented: dispatcher, decode/inspect/Review,
  prepare/attach/verify/signature_report, tx.build, tx.decode/encode,
  account/amount/message/policy, VM 40/41/44/46 registration, all three wasm
  targets.
- TODO (M2+): AST/TEX branch display refinement, VM maincall bytecode
  print/IR. WASM gzip is about 0.24MB after dropping the JS codec/adapter
  layers (under the §9 page 1MB budget).
