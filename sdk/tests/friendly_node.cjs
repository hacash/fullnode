const path = require("path");
const { pathToFileURL } = require("url");

async function run() {
    const { default: create_hacash_sdk } = await import(
        pathToFileURL(path.join(__dirname, "../dist/js/hacashsdk.mjs")).href
    );
    const sdk = await create_hacash_sdk();

    const caps = sdk.system.capabilities();
    if (caps.abi.major !== 2) {
        throw new Error(`unexpected abi major ${caps.abi.major}`);
    }
    if (!sdk.account.verify_address("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").ok) {
        throw new Error("valid address was rejected");
    }
    if (sdk.account.verify_address("2MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").ok) {
        throw new Error("invalid address was accepted");
    }
    const amount = sdk.amount.parse_protocol("0.0012");
    if (amount.value !== "12:244") {
        throw new Error(`unexpected canonical amount ${amount.value}`);
    }
    const built = sdk.tx.build({
        tx_type: 2,
        main: "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9",
        fee: "1:244",
        timestamp: 1755223764,
        actions: [
            {
                kind: "hac_transfer",
                to: "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9",
                amount: "12:244",
            },
        ],
    });
    if (!built.body || built.hash.length !== 64) {
        throw new Error("tx.build failed");
    }
    const decoded = sdk.tx.decode(built.body);
    const reencoded = sdk.tx.encode(decoded);
    if (reencoded.body !== built.body) {
        throw new Error("tx.decode → tx.encode round trip mismatch");
    }
    const review = sdk.tx.inspect_report(built.body, null);
    if (!review.review_binding) {
        throw new Error("tx.inspect_report failed");
    }

    console.log("friendly_node.cjs OK");
}

run().catch((error) => {
    console.error(error);
    process.exit(1);
});
