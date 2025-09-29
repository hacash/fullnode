const fs = require("fs")


const utilfn = `function base64ToBuffer(b) {
    let str = window.atob(b);
    let buffer = new Uint8Array(str.length);
    for (let i=0; i < str.length; i++) {
        buffer[i] = str.charCodeAt(i);
    }
    return buffer;
}`


// wasm code 2 base64
const wasmBase64  = fs.readFileSync("dist/hacashsdk_bg.wasm").toString('base64')

// replace WebAssembly.Instance
// const instanceLine = "module = new WebAssembly.Module(module);"
let wasm2jscon = fs.readFileSync("dist/hacashsdk.js").toString()
/*
    .replace(instanceLine,
    `${utilfn}\nmodule = new WebAssembly.Module(base64ToBuffer("${wasmBase64}"));`
)
*/

wasm2jscon += `
let __sdk_ok;
const hacash_sdk = async function() {
    if(!__sdk_ok) {
        await wasm_bindgen({ module_or_path: base64ToBuffer(__Hacash_WASM_SDK_Stuff)});
        __sdk_ok = true;
    }
    return wasm_bindgen
}

${utilfn}

const __Hacash_WASM_SDK_Stuff = "${wasmBase64}";
`

// output js file
fs.writeFileSync("dist/hacashsdk_bg.js", wasm2jscon)

// ok finish
