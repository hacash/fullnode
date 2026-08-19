// Unified SDK 2.0 binary ABI facade (§5).
// Underlying transport: `sdk_invoke_binary(operation_id, payload)` → binary envelope.
// - request: each operation has a fixed binary layout (W2 strings / fixed-length numbers / W4 JSON);
// - result: `ok:u8 | W4 len + JSON body` (read with native JSON.parse) or
//   `ok:u8 | err_code:u16 + W2 msg`.
// Errors throw an `SdkError`-shaped exception (code from the error-code mapping table).

import { encodeTransactionSpec, pushU16, pushU32, pushU64, pushStrW2 as pushW2Str } from "./generated/codec.mjs";
import { OP, ERROR_NAMES } from "./generated/op_tables.mjs";
import { adaptActionSpec } from "./generated/actionspec.mjs";
import { createOperationMethods } from "./generated/operations.mjs";

// ---- envelope parsing ----
// Error envelope: ok:0 | code:u16 | W2 message | W2 detail (may be empty)

function decodeEnvelope(bytes) {
    const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    if (bytes[0] === 0) {
        const code = dv.getUint16(1);
        const msgLen = dv.getUint16(3);
        const message = new TextDecoder().decode(bytes.subarray(5, 5 + msgLen));
        let detail = undefined;
        const detailLenPos = 5 + msgLen;
        if (bytes.length >= detailLenPos + 2) {
            const detailLen = dv.getUint16(detailLenPos);
            if (detailLen > 0) {
                detail = new TextDecoder().decode(
                    bytes.subarray(detailLenPos + 2, detailLenPos + 2 + detailLen),
                );
            }
        }
        return { ok: false, code, message, detail };
    }
    const len = dv.getUint32(1);
    const body = new TextDecoder().decode(bytes.subarray(5, 5 + len));
    return { ok: true, value: JSON.parse(body) };
}

// ---- binary request builders (§5) ----
// W2 = u16 length prefix; W4 = u32 length prefix; optional fields carry a
// 0/1 marker byte. Mirrors `service::ReqReader` on the Rust side. The byte
// primitives (pushU16/U32/U64/W2) come from the generated codec (single
// definition, with validation); W4 JSON and the optional wrappers live here.

function pushW4Json(out, value) {
    const bytes = Array.from(new TextEncoder().encode(JSON.stringify(value)));
    pushU32(out, bytes.length);
    out.push(...bytes);
}

function pushOptW2Str(out, value) {
    if (value === undefined || value === null) {
        out.push(0);
    } else {
        out.push(1);
        pushW2Str(out, value);
    }
}

function pushOptW4Json(out, value) {
    if (value === undefined || value === null) {
        out.push(0);
    } else {
        out.push(1);
        pushW4Json(out, value);
    }
}

function pushOptU64(out, value) {
    if (value === undefined || value === null) {
        out.push(0);
    } else {
        out.push(1);
        pushU64(out, value);
    }
}

// Optional inspect context: marker 0/1, then u64 current_height + u32
// expected_chain_id (big-endian) when present. Mirrors `service::parse_request`.
function pushOptInspectContext(out, context) {
    if (context === undefined || context === null) {
        out.push(0);
    } else {
        out.push(1);
        const hi = Math.floor(Number(context.current_height) / 0x100000000);
        const lo = Number(context.current_height) >>> 0;
        pushU32(out, hi);
        pushU32(out, lo);
        pushU32(out, Number(context.expected_chain_id) >>> 0);
    }
}

/// Error envelopes carry the numeric ABI id; map it back to the friendly
/// name (ERROR_NAMES[i] has id i+1, same positional contract as the Rust
/// `error_code_id`).
function createSdkError(code, message, detail) {
    const error = new Error(message);
    error.code = ERROR_NAMES[code - 1] ?? `unknown_${code}`;
    error.detail = detail;
    return error;
}

function createFriendlyApi(backend) {
    const invoke = (operationId, payloadBytes) => {
        const response = decodeEnvelope(
            backend.sdk_invoke_binary(operationId, new Uint8Array(payloadBytes)),
        );
        if (!response.ok) {
            throw createSdkError(response.code, response.message, response.detail);
        }
        return response.value;
    };

    // Mechanical operation methods are generated from `profile::OP_DEFS`
    // (single source: the binary request layout, parsed by the same table on
    // the Rust side). The one special operation below has a hand-written body.
    const api = createOperationMethods(invoke, {
        pushW2Str,
        pushOptW2Str,
        pushW4Json,
        pushOptW4Json,
        pushOptU64,
        pushOptInspectContext,
    });

    // `tx.build`: the SDK-interface spec (kind = "hac_transfer" etc.) goes
    // through the adapter into the wire shape (kind = "transfer_hac_to",
    // fields = wire names), then is encoded by the §4 generated codec
    // (amounts/addresses as strings; parsing stays in Rust). `timestamp`
    // defaults to the current host time.
    api.tx.build = (spec) => {
        const copy = spec && typeof spec === "object" ? { ...spec } : {};
        if (copy.timestamp === undefined || copy.timestamp === null) {
            copy.timestamp = Math.floor(Date.now() / 1000);
        }
        return invoke(
            OP.TX_BUILD,
            encodeTransactionSpec({
                ...copy,
                actions: (copy.actions ?? []).map(adaptActionSpec),
            }),
        );
    };

    return {
        transport_version: backend.sdk_transport_version(),
        ...api,
    };
}

async function loadNodeBackend() {
    return await import(new URL("../nodejs/hacashsdk.js", import.meta.url));
}

async function loadWebBackend(initInput) {
    const moduleApi = await import(new URL("../web/hacashsdk.js", import.meta.url));
    if (typeof moduleApi.default === "function") {
        if (initInput === undefined) {
            await moduleApi.default();
        } else {
            await moduleApi.default(initInput);
        }
    }
    return moduleApi;
}

/**
 * @param {{target?: "auto"|"node"|"web", wasm?: RequestInfo|URL|Response}} options
 */
export async function create_hacash_sdk(options = {}) {
    const target = options.target ?? "auto";
    const isNode =
        target === "node" ||
        (target === "auto" &&
            typeof process !== "undefined" &&
            process.versions != null &&
            process.versions.node != null);
    const backend = isNode
        ? await loadNodeBackend()
        : await loadWebBackend(options.wasm);
    return createFriendlyApi(backend);
}

export default create_hacash_sdk;
