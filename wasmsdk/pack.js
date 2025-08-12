const fs = require("fs")


const utilfn = `function base64ToBuffer(b) {
  const str = window.atob(b);
  const buffer = new Uint8Array(str.length);
  for (let i=0; i < str.length; i++) {
    buffer[i] = str.charCodeAt(i);
  }
  return buffer;
}
`

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

wasm2jscon = `${utilfn}const __Hacash_WASM_SDK_Buffer = base64ToBuffer("${wasmBase64}")\nlet hacash_sdk;\nlet hacash_sdk_mod;\n` + wasm2jscon + `
    hacash_sdk = async function() {
        if(!hacash_sdk_mod) {
            hacash_sdk_mod = await wasm_bindgen({ module_or_path: __Hacash_WASM_SDK_Buffer})
        }
        return hacash_sdk_mod
    }
`


// output js file
fs.writeFileSync("dist/hacashsdk_bg.js", wasm2jscon)

// ok finish
