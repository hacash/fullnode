const fs = require("fs");
const path = require("path");
const vm = require("vm");

const context = vm.createContext({
    console,
    WebAssembly,
    TextDecoder,
    TextEncoder,
    Uint8Array,
    ArrayBuffer,
    atob,
    btoa,
    URL,
    Request,
    Response,
    fetch,
});
context.window = context;

vm.runInContext(
    fs.readFileSync(path.join(__dirname, "../dist/page/hacashsdk_bg.js"), "utf8"),
    context,
);
vm.runInContext(
    fs.readFileSync(path.join(__dirname, "../dist/js/hacashsdk.global.js"), "utf8"),
    context,
);

(async () => {
    const sdk = await context.create_hacash_sdk();
    const account = sdk.create_account("123456");
    if (account.address !== "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9") {
        throw new Error(`unexpected account ${account.address}`);
    }
    console.log("friendly_page_vm.cjs OK");
})().catch((error) => {
    console.error(error);
    process.exit(1);
});
