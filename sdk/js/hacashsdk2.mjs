// Unified SDK 2.0 JS facade (doc 14 §9). Thin generated-style wrapper over
// the raw `sdk_invoke` transport: one operation → one JSON request → one
// envelope. Errors throw an `SdkError`-shaped exception with `code`/`detail`;
// the raw envelope is available as `e.sdkError`.

function isNodeRuntime() {
    return typeof process !== "undefined"
        && process.versions != null
        && process.versions.node != null;
}

function createSdkError(raw) {
    const error = new Error(`${raw.code}: ${raw.message}`);
    error.code = raw.code;
    error.message = raw.message;
    error.detail = raw.detail;
    error.sdkError = raw;
    return error;
}

// `tx.build` without an explicit `timestamp` gets the current host time here:
// the wasm side cannot read the clock itself on wasm32-unknown-unknown. An
// explicitly passed `timestamp` is never overwritten.
function withInjectedTimestamp(spec) {
    const copy = spec && typeof spec === "object" ? { ...spec } : {};
    if (copy.timestamp === undefined || copy.timestamp === null) {
        copy.timestamp = Math.floor(Date.now() / 1000);
    }
    return copy;
}

function createFriendlyApi(backend) {
    const invoke = (operation, payload) => {
        const request = { operation, payload };
        const response = JSON.parse(backend.sdk_invoke(JSON.stringify(request)));
        if (!response.ok) {
            throw createSdkError(response.error);
        }
        return response.value;
    };
    return {
        transport_version: backend.sdk_transport_version(),
        system: {
            capabilities: () => invoke("system.capabilities", {}),
            sdk_version: () => invoke("system.sdk_version", {}),
            codec_profile: () => invoke("system.codec_profile", {}),
        },
        tx: {
            build: (spec) => invoke("tx.build", { spec: withInjectedTimestamp(spec) }),
            inspect_report: (body, signer_address) =>
                invoke("tx.inspect_report", { body, signer_address: signer_address ?? null }),
            inspect: (body, signer_address, context) =>
                invoke("tx.inspect", { body, signer_address: signer_address ?? null, context }),
            prepare_signature: (body, signer_address, options) =>
                invoke("tx.prepare_signature", {
                    body,
                    signer_address,
                    review: options?.review ?? null,
                    policy: options?.policy ?? null,
                    origin: options?.origin ?? null,
                    expires_at: options?.expires_at ?? null,
                }),
            attach_signature: (body, proof, review, request) =>
                invoke("tx.attach_signature", {
                    body,
                    proof,
                    review,
                    request,
                }),
            attach_signature_unbound: (body, proof) =>
                invoke("tx.attach_signature_unbound", {
                    body,
                    proof,
                }),
            verify: (body) => invoke("tx.verify", { body }),
            signature_report: (body) => invoke("tx.signature_report", { body }),
            decode: (body) => invoke("tx.decode", { body }),
            encode: (transaction, review) =>
                invoke("tx.encode", {
                    transaction,
                    review: review ?? null,
                }),
        },
        account: {
            verify_address: (address) => invoke("account.verify_address", { address }),
            address_from_public_key: (public_key) =>
                invoke("account.address_from_public_key", { public_key }),
        },
        amount: {
            parse_protocol: (value) => invoke("amount.parse_protocol", { value }),
            format_protocol: (value, unit) => invoke("amount.format_protocol", { value, unit }),
        },
        message: {
            prepare_signature: (params) => invoke("message.prepare_signature", params),
            verify: (request, proof) => invoke("message.verify", { request, proof }),
        },
        policy: {
            evaluate: (review, policy) =>
                invoke("policy.evaluate", { review, policy: policy ?? {} }),
        },
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
    const mode = options.target || "auto";
    const target = mode === "auto" ? (isNodeRuntime() ? "node" : "web") : mode;
    const backend = target === "node"
        ? await loadNodeBackend()
        : await loadWebBackend(options.wasm);
    return createFriendlyApi(backend);
}

export default create_hacash_sdk;
