// Raw wire-shaped actions end-to-end: the wasm `tx.build` must accept any
// action kind the codec schema registry knows (no SDK-side filter), building
// it through the protocol's own action decoder. The chain's scope validation
// is the only judge of whether such an action belongs in a body — not the SDK.
//
//   node sdk/tests/raw_build_test.mjs   (after ./sdk/pack.sh)

import { pathToFileURL } from "url";
import path from "path";

const MAIN = "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9";

const { default: create_hacash_sdk } = await import(
    pathToFileURL(path.join(import.meta.dirname, "../dist/js/hacashsdk.mjs")).href
);
const sdk = await create_hacash_sdk();

const base = {
    tx_type: 3,
    main: MAIN,
    fee: "1:244",
    timestamp: 1755223764,
};

// A guard action the friendly adapter has no entry for: raw wire shape.
const balanceFloor = sdk.tx.build({
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
});
const decodedFloor = sdk.tx.decode(balanceFloor.body);
if (decodedFloor.actions[0].kind !== 1043) {
    throw new Error(`balance_floor built with wrong kind ${decodedFloor.actions[0].kind}`);
}

// Nested action lists (AST wrapper) go through the raw path recursively.
const astSelect = sdk.tx.build({
    ...base,
    actions: [
        {
            kind: "ast_select",
            exe_min: 1,
            exe_max: 1,
            actions: [{ kind: "transfer_hac_to", to: MAIN, hacash: "12:244" }],
        },
    ],
});
const decodedAst = sdk.tx.decode(astSelect.body);
if (decodedAst.actions[0].kind !== 25) {
    throw new Error(`ast_select built with wrong kind ${decodedAst.actions[0].kind}`);
}

// Host opcodes are exposed too; whether the chain accepts them is the chain's
// scope validation (CALL_ONLY), not the SDK's business.
const host = sdk.tx.build({
    ...base,
    actions: [{ kind: "block_height" }],
});
const decodedHost = sdk.tx.decode(host.body);
if (decodedHost.actions[0].kind !== 0x0701) {
    throw new Error(`block_height built with wrong kind ${decodedHost.actions[0].kind}`);
}

console.log("raw_build_test.mjs OK (balance_floor / ast_select / block_height via wasm)");
