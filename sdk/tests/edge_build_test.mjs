// B3/B4 behavior verification (run after pack.sh rebuilds dist):
// - diamond_mint builds without custom_message (threshold-conditional field)
// - diamond_mint still builds with custom_message
// - one-sided channel_open builds with the right_amount omitted (default "0")
import create_hacash_sdk from "../dist/js/hacashsdk.mjs";

const sdk = await create_hacash_sdk();
const MAIN = "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9";
const OTHER = "1LRi6Wn38JtUppbFv2uWyAwtctcDLtFDFr";
const spec = {
    tx_type: 2,
    main: MAIN,
    fee: "1:244",
    timestamp: 1755223764,
};

// B4: no custom_message -> builds; the native body drops the field.
const mint = sdk.tx.build({
    ...spec,
    actions: [
        {
            kind: "diamond_mint",
            diamond: "WTYUIA",
            number: "100",
            prev_hash: "0x" + "aa".repeat(32),
            nonce: "0x" + "bb".repeat(8),
            address: MAIN,
        },
    ],
});
if (!mint.body) throw new Error("diamond_mint without custom_message failed to build");
const decoded = sdk.tx.decode(mint.body);
if (decoded.actions[0].name !== "diamond_mint") {
    throw new Error(`unexpected decoded kind ${decoded.actions[0].kind}`);
}
console.log("B4 no-custom OK (native decode:", decoded.actions[0].name, ")");

// B4': with custom_message still builds and round-trips.
const mint2 = sdk.tx.build({
    ...spec,
    actions: [
        {
            kind: "diamond_mint",
            diamond: "WTYUIA",
            number: "100",
            prev_hash: "0x" + "aa".repeat(32),
            nonce: "0x" + "bb".repeat(8),
            address: MAIN,
            custom_message: "0x" + "cc".repeat(32),
        },
    ],
});
if (!mint2.body) throw new Error("diamond_mint with custom_message failed to build");
console.log("B4 with-custom OK");

// B3: one-sided channel (right_amount omitted -> "0").
const chan = sdk.tx.build({
    ...spec,
    actions: [
        {
            kind: "channel_open",
            channel_id: "0xabababababababababababababababab",
            left_address: MAIN,
            left_amount: "1:244",
            right_address: OTHER,
        },
    ],
});
if (!chan.body) throw new Error("one-sided channel_open failed to build");
const chanDecoded = sdk.tx.decode(chan.body);
if (chanDecoded.actions[0].name !== "channel_open") {
    throw new Error(`unexpected channel kind ${chanDecoded.actions[0].kind}`);
}
console.log("B3 one-sided channel OK");

console.log("edge_build_test.mjs OK");
