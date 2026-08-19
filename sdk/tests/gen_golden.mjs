// Golden-vector seed generator (one-time; the seed is committed).
//
// Renders the friendly `ActionSpec` fixtures through the GENERATED adapter +
// codec into `golden_seed.json` (friendly spec / adapted wire spec / payload
// hex). `sdk_codegen` then decodes each payload with the Rust decoder and
// writes `golden.json` (adding the `decoded` field); the Rust and JS tests
// lock both sides to the committed vectors.
//
// Usage: node sdk/tests/gen_golden.mjs
// (regenerate only when the fixture set changes; the vectors are a freeze).

import { writeFileSync } from "fs";
import { adaptActionSpec } from "../js/generated/actionspec.mjs";
import { encodeTransactionSpec, decodeTransactionSpec } from "../js/generated/codec.mjs";

const MAIN = "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9";
const MAIN2 = "1LRi6Wn38JtUppbFv2uWyAwtctcDLtFDFr";
const FEE = "1:244";
const TS = 1755223764;

const A = (kind, fields) => ({ kind, ...fields });

const actions = [
    ["hac_transfer_to", [A("hac_transfer", { to: MAIN, amount: "12:244" })]],
    ["hac_transfer_from", [A("hac_transfer", { from: MAIN2, to: MAIN, amount: "12:244" })]],
    ["sat_transfer", [A("sat_transfer", { to: MAIN, satoshi: 500 })]],
    ["sat_transfer_from", [A("sat_transfer", { from: MAIN2, to: MAIN, satoshi: 500 })]],
    ["hacd_single", [A("hacd_transfer", { to: MAIN, names: ["WTYUIA"] })]],
    ["hacd_list", [A("hacd_transfer", { to: MAIN, names: ["WTYUIA", "XVMEKB"] })]],
    ["hacd_from", [A("hacd_transfer", { from: MAIN2, to: MAIN, names: ["WTYUIA", "XVMEKB"] })]],
    ["asset_transfer", [A("asset_transfer", { to: MAIN, serial: 7, amount: "100" })]],
    ["height_scope", [A("height_scope", { start: 1_000_000, end: 0 })]],
    ["chain_allow", [A("chain_allow", { chains: [1, 2] })]],
    ["req_sign_list", [A("req_sign_list", { signers: [MAIN, MAIN2] })]],
    ["tx_message", [A("tx_message", { data: "686920686163617368" })]],
    ["tx_blob", [A("tx_blob", { data: "01020304" })]],
    ["insc_push", [A("insc_push", { diamonds: ["WTYUIA"], engraved_content: "hello", engraved_type: 2 })]],
    ["insc_clean", [A("insc_clean", { diamonds: ["WTYUIA"] })]],
    ["insc_edit", [A("insc_edit", { diamond: "WTYUIA", index: 1, engraved_content: "0x00ff" })]],
    ["insc_move", [A("insc_move", { from_diamond: "WTYUIA", to_diamond: "XVMEKB", index: 2 })]],
    ["insc_drop", [A("insc_drop", { diamond: "WTYUIA", index: 0 })]],
    [
        "channel_open",
        [
            A("channel_open", {
                channel_id: "0x" + "ab".repeat(16),
                left_address: MAIN,
                left_amount: "1:244",
                right_address: MAIN2,
                right_amount: "2:244",
            }),
        ],
    ],
    ["channel_close", [A("channel_close", { channel_id: "abcd".repeat(8) })]],
    [
        "asset_create",
        [A("asset_create", { ticket: "ticket1", name: "asset", serial: "1", supply: "100", decimal: 2, issuer: MAIN })],
    ],
    [
        "diamond_mint",
        [A("diamond_mint", { diamond: "WTYUIA", number: "100", prev_hash: "0x" + "aa".repeat(32), nonce: "0x" + "bb".repeat(8), address: MAIN, custom_message: "0x" + "cc".repeat(32) })],
    ],
    // Raw wire-shaped actions (no friendly adapter entry): the generic
    // `RawAction` path must build them, so the codec round-trip is locked too.
    [
        "raw_balance_floor",
        [A("balance_floor", { addr: MAIN, hacash: "12:244", satoshi: "100", diamond: "5", assets: [{ serial: "7", amount: "100" }] })],
    ],
    [
        "raw_contract_main_call",
        [A("contract_main_call", { marks: "000000", codeconf: 1, codes: "010203" })],
    ],
    [
        "raw_ast_select",
        [A("ast_select", { exe_min: 1, exe_max: 1, actions: [{ kind: "transfer_hac_to", to: MAIN, hacash: "12:244" }] })],
    ],
    ["raw_block_height", [A("block_height", {})]],
];

function canon(value) {
    if (Array.isArray(value)) return value.map(canon);
    if (value !== null && typeof value === "object") {
        const out = {};
        for (const key of Object.keys(value).sort()) out[key] = canon(value[key]);
        return out;
    }
    return typeof value === "number" ? String(value) : value;
}

const vectors = [];
for (const [name, friendlyActions] of actions) {
    const friendly = { tx_type: 2, main: MAIN, fee: FEE, timestamp: TS, gas_max: 0, actions: friendlyActions };
    const wireActions = friendlyActions.map((a) => adaptActionSpec({ ...a }));
    const wire = { ...friendly, actions: wireActions };
    const payload = Buffer.from(encodeTransactionSpec(wire)).toString("hex");
    // sanity: the codec decodes its own payload (design A normalizes numbers
    // to strings, so compare canonically)
    const back = decodeTransactionSpec(Buffer.from(payload, "hex"));
    if (JSON.stringify(canon(back.actions)) !== JSON.stringify(canon(wireActions))) {
        throw new Error(`codec round-trip mismatch for ${name}`);
    }
    vectors.push({ name, friendly, wire, payload });
}

writeFileSync(
    new URL("../tests/golden_seed.json", import.meta.url),
    JSON.stringify({ vectors }, null, 2) + "\n",
);
console.log(`gen_golden: wrote golden_seed.json (${vectors.length} vectors)`);
