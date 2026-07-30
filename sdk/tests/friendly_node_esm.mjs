import create_hacash_sdk from "../dist/js/hacashsdk.mjs";

const sdk = await create_hacash_sdk();
const account = sdk.create_account("123456");

if (!account.address) {
    throw new Error("create_account failed in esm");
}
if (typeof sdk.hac_to_mei("1:244") !== "number") {
    throw new Error("hac_to_mei failed in esm");
}

console.log("friendly_node_esm.mjs OK");
