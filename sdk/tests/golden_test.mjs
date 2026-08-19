// Golden vectors, TS side (no wasm needed): the GENERATED adapter + codec must
// reproduce the committed `golden.json` vectors exactly.
//
//   node sdk/tests/golden_test.mjs
//
// Each vector locks:
// - forward:  adaptActionSpec(friendly) == wire, encodeTransactionSpec(wire) == payload
// - reverse:  decodeTransactionSpec(payload) == wire (canonicalized)

import { readFileSync } from "fs";
import { adaptActionSpec } from "../js/generated/actionspec.mjs";
import { encodeTransactionSpec, decodeTransactionSpec } from "../js/generated/codec.mjs";

const golden = JSON.parse(
    readFileSync(new URL("./golden.json", import.meta.url), "utf8"),
);

function canon(value) {
    if (Array.isArray(value)) return value.map(canon);
    if (value !== null && typeof value === "object") {
        const out = {};
        for (const key of Object.keys(value).sort()) out[key] = canon(value[key]);
        return out;
    }
    return typeof value === "number" ? String(value) : value;
}

let checked = 0;
for (const vector of golden.vectors) {
    const { name, friendly, wire, payload } = vector;

    // forward: the generated adapter reproduces the wire shape
    const adapted = friendly.actions.map((a) => adaptActionSpec({ ...a }));
    if (JSON.stringify(canon(adapted)) !== JSON.stringify(canon(wire.actions))) {
        throw new Error(`${name}: adaptActionSpec output != wire`);
    }
    // forward: the generated codec reproduces the payload bytes
    const encoded = Buffer.from(encodeTransactionSpec(wire)).toString("hex");
    if (encoded !== payload) {
        throw new Error(`${name}: encodeTransactionSpec output != payload`);
    }
    // reverse: the codec decodes back to the wire shape (canonical)
    const decoded = decodeTransactionSpec(Buffer.from(payload, "hex"));
    if (JSON.stringify(canon(decoded.actions)) !== JSON.stringify(canon(wire.actions))) {
        throw new Error(`${name}: decodeTransactionSpec output != wire`);
    }
    checked += 1;
}
console.log(`golden_test.mjs OK (${checked} vectors, forward+reverse)`);
