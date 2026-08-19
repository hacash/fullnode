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
    // The no-modules bundle exposes the raw binary transport (§5): one
    // (operation_id, payload) call in, one binary envelope out.
    const raw = await context.hacash_sdk();
    if (
        typeof raw.sdk_invoke_binary !== "function" ||
        raw.sdk_transport_version() !== 3
    ) {
        throw new Error("raw transport exports missing");
    }
    const invoke = (operation, payload) => {
        const bytes = raw.sdk_invoke_binary(operation, new Uint8Array(payload));
        if (bytes[0] === 0) {
            const code = (bytes[1] << 8) | bytes[2];
            const msgLen = (bytes[3] << 8) | bytes[4];
            const message = new TextDecoder().decode(bytes.subarray(5, 5 + msgLen));
            throw new Error(`sdk error ${code}: ${message}`);
        }
        const len =
            (bytes[1] << 24) | (bytes[2] << 16) | (bytes[3] << 8) | bytes[4];
        return JSON.parse(new TextDecoder().decode(bytes.subarray(5, 5 + len)));
    };
    const caps = invoke(1 /* SYSTEM_CAPABILITIES */, []);
    if (caps.abi.major !== 2) {
        throw new Error(`capabilities failed: ${JSON.stringify(caps)}`);
    }
    const address = "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9";
    const addrBytes = Array.from(new TextEncoder().encode(address));
    const check = invoke(14 /* ACCOUNT_VERIFY_ADDRESS */, [
        (addrBytes.length >> 8) & 0xff,
        addrBytes.length & 0xff,
        ...addrBytes,
    ]);
    if (check.ok !== true) {
        throw new Error(`verify_address failed: ${JSON.stringify(check)}`);
    }
    console.log("friendly_page_vm.cjs OK");
})().catch((error) => {
    console.error(error);
    process.exit(1);
});
