const create_hacash_sdk = require("../dist/js/hacashsdk.cjs");

async function run() {
    const sdk = await create_hacash_sdk();
    if (sdk.to_u64_bigint("timestamp", "18446744073709551615") !== 18446744073709551615n) {
        throw new Error("uint64 string conversion failed");
    }
    try {
        sdk.to_u64_bigint("timestamp", Number.MAX_SAFE_INTEGER + 1);
        throw new Error("unsafe JS integer was accepted");
    } catch (error) {
        if (!String(error.message || error).includes("safe integer")) {
            throw error;
        }
    }
    try {
        sdk.hac_to_mei("0.0.012");
        throw new Error("invalid amount was accepted");
    } catch (error) {
        if (!String(error).includes("amount value invalid")) {
            throw error;
        }
    }
    if (sdk.verify_address("2MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").ok) {
        throw new Error("invalid address was accepted");
    }
    const account = sdk.create_account("123456");
    const transaction = sdk.create_coin_transfer({
        main_prikey: "123456",
        to_address: "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9",
        fee: "1:244",
        hacash: "12.0",
        satoshi: "12000000",
        timestamp: 1755223764,
        chain_id: 0,
    });

    if (account.address !== "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9") {
        throw new Error(`unexpected account ${account.address}`);
    }
    if (transaction.hash !== "0b6f0b86427acc0834805a517f7fb943a38ae98d0deb52beeaa86f82679323c2") {
        throw new Error(`legacy transaction mismatch ${transaction.hash}`);
    }

    const signed = sdk.sign_transaction({
        prikey: "123456",
        body: transaction.body,
    });
    if (signed.body !== transaction.body) {
        throw new Error("sign_transaction changed an already signed transaction");
    }

    console.log("friendly_node.cjs OK");
}

run().catch((error) => {
    console.error(error);
    process.exit(1);
});
