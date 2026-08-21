# Hacash Unified SDK 2.0

The fullnode WASM SDK rebuilt under Unified SDK 2.0 (doc 14 `unified-sdk-major-version-design.md`). Design points:

- **No v1/v2 namespaces**: one surface, one release line; capabilities are
  expressed by the feature/schema/profile of `system.capabilities()`.
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
const envelope = sdk.sdk_invoke_json(4, { spec });
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
`system.capabilities().features` and the dispatcher share the same
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

- Implemented: dispatcher, codec_profile/capabilities, decode/inspect/Review,
  prepare/attach/verify/signature_report, tx.build, tx.decode/encode,
  account/amount/message/policy, VM 40/41/44/46 registration, all three wasm
  targets.
- TODO (M2+): AST/TEX branch display refinement, VM maincall bytecode
  print/IR. WASM gzip is about 0.24MB after dropping the JS codec/adapter
  layers (under the §9 page 1MB budget).
