// Wire-shaped actions end-to-end: the wasm `tx.build` accepts any action
// kind the SDK codec catalog knows. The chain's scope validation is the only
// judge of whether such an action belongs in a body — not the SDK.
//
//   node sdk/tests/raw_build_test.mjs   (after ./sdk/pack.sh)

import { pathToFileURL } from "url";
import path from "path";

const MAIN = "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9";

const { default: create_hacash_sdk } = await import(
    pathToFileURL(path.join(import.meta.dirname, "../dist/js/hacashsdk.mjs")).href
);
const sdk = await create_hacash_sdk();
const invoke = (operationId, payload) => {
    const envelope = sdk.sdk_invoke_json(operationId, payload);
    if (envelope.ok !== 1) {
        throw new Error(`SDK ${operationId} failed: ${envelope.msg}`);
    }
    return envelope.body;
};

const base = {
    tx_type: 3,
    main: MAIN,
    fee: "1:244",
    timestamp: 1755223764,
};

const balanceFloor = invoke(4, { spec: {
    ...base,
    actions: [
        {
            kind: "balance_floor",
            addr: MAIN,
            hacash: "12:244",
            satoshi: "100",
            diamond: "5",
            assets: [{ serial: "7", amount: "100" }],
        },
    ],
} });
const decodedFloor = invoke(12, { body: balanceFloor.body });
if (decodedFloor.actions[0].name !== "balance_floor") {
    throw new Error(`balance_floor built with wrong name ${decodedFloor.actions[0].name}`);
}

const astSelect = invoke(4, { spec: {
    ...base,
    actions: [
        {
            kind: "ast_select",
            exe_min: 1,
            exe_max: 1,
            actions: [{ kind: "transfer_hac_to", to: MAIN, hacash: "12:244" }],
        },
    ],
} });
const decodedAst = invoke(12, { body: astSelect.body });
if (decodedAst.actions[0].name !== "ast_select") {
    throw new Error(`ast_select built with wrong name ${decodedAst.actions[0].name}`);
}

try {
    const envelope = sdk.sdk_invoke_json(4, { spec: {
        ...base,
        actions: [{ kind: "block_height" }],
    } });
    if (envelope.ok !== 0 || envelope.code !== 5) {
        throw new Error(`unexpected invalid-action result ${JSON.stringify(envelope)}`);
    }
} catch (error) {
    throw error;
}

console.log("raw_build_test.mjs OK (SDK action subset enforced via wasm)");
