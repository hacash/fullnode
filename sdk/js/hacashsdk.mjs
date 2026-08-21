// Raw JSON WASM facade. It only loads the backend, JSON-encodes requests and
// JSON-decodes the Rust envelope. Operation ids, request fields, and response
// shapes are the Rust/WASM ABI directly; this module does not define a domain
// API, translate errors, or adapt transaction/action objects.

const TE = new TextEncoder();

function createApi(backend) {
    const sdk_invoke_json = (operationId, payload = {}) => {
        const requestBytes = TE.encode(JSON.stringify(payload ?? {}));
        return JSON.parse(backend.sdk_invoke_json(operationId, requestBytes));
    };

    return {
        sdk_invoke_json,
        sdk_transport_version: () => backend.sdk_transport_version(),
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
    return createApi(backend);
}

export default create_hacash_sdk;
