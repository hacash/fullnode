(function(root) {
    const U64_MAX = (1n << 64n) - 1n;

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

    function ensureObjectParam(name, value) {
        if (value === undefined || value === null) {
            return {};
        }
        if (typeof value !== "object") {
            throw new TypeError(`${name} expects an object or wasm-bindgen class instance`);
        }
        return value;
    }

    function createFriendlyApi(rawApi) {
        if (typeof rawApi.create_account !== "function"
            || typeof rawApi.create_coin_transfer !== "function"
            || typeof rawApi.sign_transaction !== "function") {
            throw new Error("invalid hacash sdk backend exports");
        }

        const create_coin_transfer_param = (input) => {
            if (typeof rawApi.CoinTransferParam === "function"
                && input instanceof rawApi.CoinTransferParam) {
                return input;
            }
            const source = ensureObjectParam("create_coin_transfer_param", input);
            const param = new rawApi.CoinTransferParam();
            for (const field of [
                "main_prikey",
                "from_prikey",
                "fee",
                "to_address",
                "hacash",
                "diamonds",
            ]) {
                if (Object.prototype.hasOwnProperty.call(source, field)
                    && source[field] !== undefined
                    && source[field] !== null) {
                    param[field] = String(source[field]);
                }
            }
            for (const field of ["timestamp", "satoshi", "chain_id"]) {
                if (Object.prototype.hasOwnProperty.call(source, field)
                    && source[field] !== undefined
                    && source[field] !== null) {
                    param[field] = toU64BigInt(field, source[field]);
                }
            }
            return param;
        };

        const create_sign_tx_param = (input) => {
            if (typeof rawApi.SignTxParam === "function" && input instanceof rawApi.SignTxParam) {
                return input;
            }
            const source = ensureObjectParam("create_sign_tx_param", input);
            const param = new rawApi.SignTxParam();
            for (const field of ["prikey", "body"]) {
                if (Object.prototype.hasOwnProperty.call(source, field)
                    && source[field] !== undefined
                    && source[field] !== null) {
                    param[field] = String(source[field]);
                }
            }
            return param;
        };

        return {
            env: "browser-global",
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

    async function create_hacash_sdk() {
        if (typeof root.hacash_sdk !== "function") {
            throw new Error("hacash_sdk not found, load page/hacashsdk_bg.js first");
        }
        return createFriendlyApi(await root.hacash_sdk());
    }

    root.hacash_sdk_api = root.hacash_sdk_api || {};
    root.hacash_sdk_api.create_hacash_sdk = create_hacash_sdk;
    root.create_hacash_sdk = create_hacash_sdk;
})(typeof globalThis !== "undefined" ? globalThis : window);
