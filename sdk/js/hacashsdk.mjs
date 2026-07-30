const U64_MAX = (1n << 64n) - 1n;

function isNodeRuntime() {
    return typeof process !== "undefined"
        && process.versions != null
        && process.versions.node != null;
}

function normalizeTarget(target) {
    const mode = target || "auto";
    if (mode !== "auto" && mode !== "node" && mode !== "web") {
        throw new Error(`unsupported target "${mode}", expected auto|node|web`);
    }
    return mode;
}

function pickBackendApi(moduleApi) {
    if (moduleApi && typeof moduleApi.create_account === "function") {
        return moduleApi;
    }
    if (moduleApi && moduleApi.default && typeof moduleApi.default.create_account === "function") {
        return moduleApi.default;
    }
    throw new Error("invalid hacash sdk backend: create_account not found");
}

function pickField(input, names) {
    for (const name of names) {
        if (Object.prototype.hasOwnProperty.call(input, name) && input[name] !== undefined) {
            return input[name];
        }
    }
    return undefined;
}

function toU64BigInt(field, value) {
    let number = value;
    if (typeof number === "number") {
        if (!Number.isSafeInteger(number)) {
            throw new TypeError(`${field} must be a safe integer or use string|bigint`);
        }
        number = BigInt(number);
    } else if (typeof number === "string") {
        let text = number.trim();
        if (text.endsWith("n")) {
            text = text.slice(0, -1);
        }
        if (!/^\d+$/.test(text)) {
            throw new TypeError(`${field} must be a uint64 string`);
        }
        number = BigInt(text);
    } else if (typeof number !== "bigint") {
        throw new TypeError(`${field} must be number|string|bigint`);
    }
    if (number < 0n || number > U64_MAX) {
        throw new RangeError(`${field} out of uint64 range`);
    }
    return number;
}

function isInstanceOf(value, klass) {
    return typeof klass === "function" && value instanceof klass;
}

function ensureObjectParam(name, value) {
    if (value === undefined || value === null) {
        return {};
    }
    if (typeof value !== "object") {
        throw new TypeError(`${name} expects an object or wasm-bindgen class instance`);
    }
    return value;
}

function createFriendlyApi(rawApi, env) {
    if (typeof rawApi.create_account !== "function"
        || typeof rawApi.create_coin_transfer !== "function"
        || typeof rawApi.sign_transaction !== "function") {
        throw new Error("invalid hacash sdk backend exports");
    }

    const create_coin_transfer_param = (input) => {
        if (isInstanceOf(input, rawApi.CoinTransferParam)) {
            return input;
        }
        const source = ensureObjectParam("create_coin_transfer_param", input);
        const param = new rawApi.CoinTransferParam();
        const stringFields = [
            "main_prikey",
            "from_prikey",
            "fee",
            "to_address",
            "hacash",
            "diamonds",
        ];
        for (const field of stringFields) {
            const value = pickField(source, [field]);
            if (value !== undefined && value !== null) {
                param[field] = String(value);
            }
        }
        for (const field of ["timestamp", "satoshi", "chain_id"]) {
            const value = pickField(source, [field]);
            if (value !== undefined && value !== null) {
                param[field] = toU64BigInt(field, value);
            }
        }
        return param;
    };

    const create_sign_tx_param = (input) => {
        if (isInstanceOf(input, rawApi.SignTxParam)) {
            return input;
        }
        const source = ensureObjectParam("create_sign_tx_param", input);
        const param = new rawApi.SignTxParam();
        for (const field of ["prikey", "body"]) {
            const value = pickField(source, [field]);
            if (value !== undefined && value !== null) {
                param[field] = String(value);
            }
        }
        return param;
    };

    return {
        env,
        raw: rawApi,
        account_class: rawApi.Account,
        coin_transfer_param_class: rawApi.CoinTransferParam,
        coin_transfer_result_class: rawApi.CoinTransferResult,
        sign_tx_param_class: rawApi.SignTxParam,
        sign_tx_result_class: rawApi.SignTxResult,
        verify_address_result_class: rawApi.VerifyAddressResult,
        to_u64_bigint: (field, value) => toU64BigInt(field, value),
        create_coin_transfer_param,
        create_sign_tx_param,
        create_account: rawApi.create_account,
        hac_to_unit: rawApi.hac_to_unit,
        hac_to_mei: rawApi.hac_to_mei,
        verify_address: rawApi.verify_address,
        create_coin_transfer: (input) => rawApi.create_coin_transfer(create_coin_transfer_param(input)),
        sign_transaction: (input) => rawApi.sign_transaction(create_sign_tx_param(input)),
    };
}

async function loadNodeBackend() {
    const moduleApi = await import(new URL("../nodejs/hacashsdk.js", import.meta.url));
    return pickBackendApi(moduleApi);
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
    return pickBackendApi(moduleApi);
}

export async function create_hacash_sdk(options = {}) {
    const mode = normalizeTarget(options.target);
    const nodeRuntime = isNodeRuntime();
    const target = mode === "auto" ? (nodeRuntime ? "node" : "web") : mode;
    if (target === "node" && !nodeRuntime) {
        throw new Error("node target requested in non-node runtime");
    }
    const backend = target === "node"
        ? await loadNodeBackend()
        : await loadWebBackend(options.web_init_input ?? options.wasm);
    return createFriendlyApi(backend, target);
}

export default create_hacash_sdk;
