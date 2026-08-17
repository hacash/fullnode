import create_hacash_sdk from "../dist/js/hacashsdk.mjs";

const sdk = await create_hacash_sdk();

const caps = sdk.system.capabilities();
if (caps.abi.major !== 2) {
    throw new Error(`unexpected abi major ${caps.abi.major}`);
}
if (sdk.transport_version !== 1) {
    throw new Error("unexpected transport version");
}
// format_protocol returns an exact decimal string, never a float.
const formatted = sdk.amount.format_protocol("12:244", 248);
if (formatted !== "0.0012" || typeof formatted !== "string") {
    throw new Error(`unexpected format_protocol result ${formatted}`);
}

console.log("friendly_node_esm.mjs OK");
