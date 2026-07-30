const fs = require("fs");
const path = require("path");

const base64ToBuffer = `function base64ToBuffer(base64) {
    const text = window.atob(base64);
    const buffer = new Uint8Array(text.length);
    for (let i = 0; i < text.length; i++) {
        buffer[i] = text.charCodeAt(i);
    }
    return buffer;
}`;

const distDir = path.join(__dirname, "dist");
const wasmBase64 = fs
    .readFileSync(path.join(distDir, "hacashsdk_bg.wasm"))
    .toString("base64");
let output = fs.readFileSync(path.join(distDir, "hacashsdk.js")).toString();

output += `
let __sdk_ok;
globalThis.hacash_sdk = async function() {
    if (!__sdk_ok) {
        await wasm_bindgen({ module_or_path: base64ToBuffer(__Hacash_WASM_SDK_Stuff) });
        __sdk_ok = true;
    }
    return wasm_bindgen;
};

${base64ToBuffer}

const __Hacash_WASM_SDK_Stuff = "${wasmBase64}";
`;

fs.writeFileSync(path.join(distDir, "hacashsdk_bg.js"), output);
