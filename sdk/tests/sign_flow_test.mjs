// Full approval-chain flow through the wasm (run after ./sdk/pack.sh):
// build → inspect → prepare → attach → verify, with a golden vault proof.
// Exercises the GENERATED bjson codec (bjson_codec.mjs) on both directions:
// the request side (encodeReview/encodeSigningRequest/encodeSignatureProof)
// and the response side (decodeReview/decodeSigningRequest/decodeTransactionJson).
//
// The proof is a golden vector: private key "123456" (the same vault the Rust
// flow tests use), signature produced by the Rust signer (sys::Account) over
// the deterministic sign hash of the fixed body below — identical in spirit
// to the golden `LEGACY_BODY` in flow.rs. The JS side never holds keys.
import create_hacash_sdk from "../dist/js/hacashsdk.mjs";

const MAIN = "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9";
const GOLDEN_DIGEST = "12a67a38925da824d9b2706d3abc2d2b2ae861fff5ade2fa8ef4430198ccad25";
const GOLDEN_PUBLIC_KEY = "0231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263c";
const GOLDEN_SIGNATURE =
    "2c4774f915a987d57285bb67e8efd8ec37cbe11373ef4ba07aa48a47459306a8" +
    "14517d378531649a359e8e864067f2cb6e957953147424b9fa132aff2a017197";

const sdk = await create_hacash_sdk();

const built = sdk.tx.build({
    tx_type: 2,
    main: MAIN,
    fee: "1:244",
    timestamp: 1755223764,
    actions: [{ kind: "hac_transfer", to: MAIN, amount: "12:244" }],
});

const review = sdk.tx.inspect(built.body, MAIN, {
    current_height: 1000000,
    expected_chain_id: 0,
});
if (review.protocol_valid !== true || review.actions.length !== 1) {
    throw new Error(`unexpected review ${JSON.stringify(review)}`);
}

const request = sdk.tx.prepare_signature(built.body, MAIN, { review });
if (request.digest !== GOLDEN_DIGEST) {
    throw new Error(`digest drifted from the golden vector: ${request.digest}`);
}
if (request.id !== request.request_binding) {
    throw new Error("prepare_signature output malformed");
}

const proof = {
    schema: "hacash.sdk/signature-proof@1",
    request_id: request.id,
    request_binding: request.request_binding,
    public_key: GOLDEN_PUBLIC_KEY,
    signature: GOLDEN_SIGNATURE,
    algorithm: "secp256k1-rfc6979-sha256",
};

const attached = sdk.tx.attach_signature(built.body, proof, review, request);
if (!attached.complete || attached.missing_signers.length !== 0) {
    throw new Error(`attach incomplete: ${JSON.stringify(attached)}`);
}

const verified = sdk.tx.verify(attached.body);
if (!verified.ok) {
    throw new Error(`attached body does not verify: ${JSON.stringify(verified)}`);
}

// tx.decode round-trip through the generated decodeTransactionJson.
const decoded = sdk.tx.decode(attached.body);
if (decoded.actions.length !== 1 || decoded.signatures.length !== 1) {
    throw new Error(`unexpected decode output ${JSON.stringify(decoded)}`);
}

console.log("sign_flow_test.mjs OK (build→inspect→prepare→attach→verify via generated bjson codec)");
