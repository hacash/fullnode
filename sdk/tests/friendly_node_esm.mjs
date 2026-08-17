import create_hacash_sdk from "../dist/js/hacashsdk.mjs";

const sdk = await create_hacash_sdk();

const caps = sdk.system.capabilities();
if (caps.abi.major !== 2) {
    throw new Error(`unexpected abi major ${caps.abi.major}`);
}
if (sdk.transport_version !== 1) {
    throw new Error("unexpected transport version");
}
const formatted = sdk.amount.format_protocol("12:244", 8);
if (typeof formatted !== "number") {
    throw new Error("amount.format_protocol failed in esm");
}

console.log("friendly_node_esm.mjs OK");
