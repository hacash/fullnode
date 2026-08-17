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

(async () => {
    // The no-modules bundle exposes the raw transport: one JSON request in,
    // one envelope JSON out. No global facade exists anymore (v2 surface).
    const raw = await context.hacash_sdk();
    if (typeof raw.sdk_invoke !== "function" || raw.sdk_transport_version() !== 1) {
        throw new Error("raw transport exports missing");
    }
    const invoke = (operation, payload) =>
        JSON.parse(raw.sdk_invoke(JSON.stringify({ operation, payload })));
    const response = invoke("system.capabilities", {});
    if (!response.ok || response.value.abi.major !== 2) {
        throw new Error(`capabilities failed: ${JSON.stringify(response)}`);
    }
    const check = invoke("account.verify_address", {
        address: "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9",
    });
    if (!check.ok || !check.value.ok) {
        throw new Error("verify_address failed");
    }
    console.log("friendly_page_vm.cjs OK");
})().catch((error) => {
    console.error(error);
    process.exit(1);
});
