// Unified SDK 2.0 binary ABI facade (§5).
// Underlying transport: `sdk_invoke_binary(operation_id, payload)` → binary envelope.
// - request: each operation has a fixed binary layout (W2 strings / fixed-length
//   numbers / W4 `bjson` field streams for complex objects);
// - result: `ok:u8 | W4 len + body` where body is a `bjson` field stream decoded
//   per operation into the response object, or `ok:u8 | err_code:u16 + W2 msg + W2 detail`.
// The wasm core is JSON-free: all JSON belongs to this JS layer.
// Errors throw an `SdkError`-shaped exception (code from the error-code mapping table).
// The complex-object pack/unpack functions are GENERATED (bjson_codec.mjs) from
// the Rust `sdk::json::BIN_TYPES` layouts; this file only wires the envelope.

import { encodeTransactionSpec, pushU8, pushU16, pushU32, pushU64, pushStrW2 as pushW2Str } from "./generated/codec.mjs";
import { OP, ERROR_NAMES } from "./generated/op_tables.mjs";
import { adaptActionSpec } from "./generated/actionspec.mjs";
import { createOperationMethods } from "./generated/operations.mjs";
import {
    encodeSignatureProof,
    encodeReview,
    encodeSigningRequest,
    encodePolicy,
    encodeTransactionJson,
    encodeMessagePrepareParams,
    decodeReview,
    decodeSigningRequest,
    decodePolicyDecision,
    decodeSignatureReport,
    decodeTransactionJson,
    decodeBuiltTransaction,
    decodeCapabilities,
    decodeCodecProfile,
    decodeSdkVersion,
    decodeAttachResult,
    decodeVerifyResult,
    decodeVerifyAddressResult,
    decodeAddressFromPublicKeyResult,
    decodeParsedAmount,
    decodeMessageVerifyResult,
    decodeAmountFormatResult,
} from "./generated/bjson_codec.mjs";

const TE = new TextEncoder();
const TD = new TextDecoder();

function pushW4Bin(out, body) {
    pushU32(out, body.length);
    out.push(...body);
}
function pushOptBin(out, encoder, value) {
    if (value === undefined || value === null) {
        out.push(0);
    } else {
        out.push(1);
        // encoder is an `(out, v)` push helper (it writes the W4 bin itself).
        encoder(out, value);
    }
}
function pushOptW2Str(out, value) {
    if (value === undefined || value === null) {
        out.push(0);
    } else {
        out.push(1);
        pushW2Str(out, value);
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
        pushU64(out, context.current_height);
        pushU32(out, context.expected_chain_id);
    }
}

// ================================ bjson field-stream reader ================================

function readFieldStream(bytes) {
    const fields = new Map();
    let pos = 0;
    const take = (n) => {
        if (!Number.isSafeInteger(n) || n < 0 || pos + n > bytes.length) {
            throw new Error("malformed binary response: truncated field stream");
        }
        const value = bytes.subarray(pos, pos + n);
        pos += n;
        return value;
    };
    const u8 = () => take(1)[0];
    const u16 = () => {
        const b = take(2);
        const v = (b[0] << 8) | b[1];
        return v;
    };
    const u32 = () => {
        const b = take(4);
        const v = ((b[0] << 24) | (b[1] << 16) | (b[2] << 8) | b[3]) >>> 0;
        return v;
    };
    const u64 = () => {
        const hi = u32();
        const lo = u32();
        const value = (BigInt(hi) << 32n) | BigInt(lo);
        return value <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(value) : value;
    };
    const str = () => {
        const len = u32();
        const s = TD.decode(take(len));
        return s;
    };
    const u64Arr = () => {
        const n = u32();
        if (n > Math.floor((bytes.length - pos) / 8)) throw new Error("malformed binary response: array count exceeds payload");
        const a = [];
        for (let i = 0; i < n; i++) a.push(u64());
        return a;
    };
    const u32Arr = () => {
        const n = u32();
        if (n > Math.floor((bytes.length - pos) / 4)) throw new Error("malformed binary response: array count exceeds payload");
        const a = [];
        for (let i = 0; i < n; i++) a.push(u32());
        return a;
    };
    const strArr = () => {
        const n = u32();
        if (n > Math.floor((bytes.length - pos) / 4)) throw new Error("malformed binary response: array count exceeds payload");
        const a = [];
        for (let i = 0; i < n; i++) a.push(str());
        return a;
    };
    const obj = () => {
        const len = u32();
        const inner = take(len);
        return readFieldStream(inner);
    };
    const objArr = () => {
        const n = u32();
        if (n > Math.floor((bytes.length - pos) / 4)) throw new Error("malformed binary response: array count exceeds payload");
        const a = [];
        for (let i = 0; i < n; i++) {
            const len = u32();
            const inner = take(len);
            a.push(readFieldStream(inner));
        }
        return a;
    };
    while (pos < bytes.length) {
        const nameLen = u16();
        const name = TD.decode(take(nameLen));
        const tag = u8();
        switch (String.fromCharCode(tag)) {
            case "s": fields.set(name, str()); break;
            case "u": fields.set(name, u64()); break;
            case "i": fields.set(name, u32()); break;
            case "t": fields.set(name, u8()); break;
            case "b": {
                const value = u8();
                if (value !== 0 && value !== 1) throw new Error("malformed binary response: invalid bool");
                fields.set(name, value === 1);
                break;
            }
            case "a": fields.set(name, strArr()); break;
            case "A": fields.set(name, u64Arr()); break;
            case "B": fields.set(name, u32Arr()); break;
            case "o": fields.set(name, obj()); break;
            case "O": fields.set(name, objArr()); break;
            case "n": fields.set(name, null); break;
            default: throw new Error(`unknown bjson tag ${tag}`);
        }
    }
    return fields;
}

// ================================ per-operation response decoders ================================
// The decode functions themselves are GENERATED from the Rust `BIN_TYPES`
// layouts (bjson_codec.mjs); only the operation → response-type mapping lives
// here (one line per operation, added with the operation).

const RESPONSE_DECODERS = {
    [OP.SYSTEM_CAPABILITIES]: decodeCapabilities,
    [OP.SYSTEM_SDK_VERSION]: decodeSdkVersion,
    [OP.SYSTEM_CODEC_PROFILE]: decodeCodecProfile,
    [OP.TX_BUILD]: decodeBuiltTransaction,
    [OP.TX_ENCODE]: decodeBuiltTransaction,
    [OP.TX_INSPECT_REPORT]: decodeReview,
    [OP.TX_INSPECT]: decodeReview,
    [OP.TX_DECODE]: decodeTransactionJson,
    [OP.TX_PREPARE_SIGNATURE]: decodeSigningRequest,
    [OP.MESSAGE_PREPARE_SIGNATURE]: decodeSigningRequest,
    [OP.TX_ATTACH_SIGNATURE]: decodeAttachResult,
    [OP.TX_ATTACH_SIGNATURE_UNBOUND]: decodeAttachResult,
    [OP.TX_VERIFY]: decodeVerifyResult,
    [OP.TX_SIGNATURE_REPORT]: decodeSignatureReport,
    [OP.ACCOUNT_VERIFY_ADDRESS]: decodeVerifyAddressResult,
    [OP.ACCOUNT_ADDRESS_FROM_PUBLIC_KEY]: decodeAddressFromPublicKeyResult,
    [OP.AMOUNT_PARSE_PROTOCOL]: decodeParsedAmount,
    [OP.AMOUNT_FORMAT_PROTOCOL]: decodeAmountFormatResult,
    [OP.MESSAGE_VERIFY]: decodeMessageVerifyResult,
    [OP.POLICY_EVALUATE]: decodePolicyDecision,
};

// ---- envelope parsing ----
// Error envelope: ok:0 | code:u16 | W2 message | W2 detail (may be empty)

function decodeEnvelope(bytes, operationId) {
    if (!(bytes instanceof Uint8Array) || bytes.length < 1) {
        throw new Error("malformed binary response: missing envelope");
    }
    const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    if (bytes[0] === 0) {
        if (bytes.length < 5) throw new Error("malformed binary response: truncated error envelope");
        const code = dv.getUint16(1);
        const msgLen = dv.getUint16(3);
        const msgEnd = 5 + msgLen;
        if (msgEnd + 2 > bytes.length) throw new Error("malformed binary response: truncated error message");
        const message = TD.decode(bytes.subarray(5, msgEnd));
        let detail = undefined;
        const detailLen = dv.getUint16(msgEnd);
        const detailEnd = msgEnd + 2 + detailLen;
        if (detailEnd !== bytes.length) {
            throw new Error("malformed binary response: invalid error detail length");
        }
        if (detailLen > 0) detail = TD.decode(bytes.subarray(msgEnd + 2, detailEnd));
        return { ok: false, code, message, detail };
    }
    if (bytes[0] !== 1 || bytes.length < 5) {
        throw new Error("malformed binary response: invalid success envelope");
    }
    const len = dv.getUint32(1);
    if (len !== bytes.length - 5) {
        throw new Error("malformed binary response: invalid body length");
    }
    const body = bytes.subarray(5);
    const decoder = RESPONSE_DECODERS[operationId];
    if (decoder === undefined) {
        throw new Error(`no response decoder for operation ${operationId}`);
    }
    return { ok: true, value: decoder(readFieldStream(body)) };
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
            operationId,
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
        pushProofBin: (out, v) => pushW4Bin(out, encodeSignatureProof(v)),
        pushReviewBin: (out, v) => pushW4Bin(out, encodeReview(v)),
        pushRequestBin: (out, v) => pushW4Bin(out, encodeSigningRequest(v)),
        pushPolicyBin: (out, v) => pushW4Bin(out, encodePolicy(v)),
        pushTransactionBin: (out, v) => pushW4Bin(out, encodeTransactionJson(v)),
        pushParamsBin: (out, v) => pushW4Bin(out, encodeMessagePrepareParams(v)),
        pushOptBin,
        pushOptU64,
        pushOptInspectContext,
        pushU8,
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

    // `amount.format_protocol` historically returns a bare decimal string
    // (the response body carries `{value}`); unwrap to keep the API stable.
    {
        const generated = api.amount.format_protocol;
        api.amount.format_protocol = (...args) => generated(...args).value;
    }

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
