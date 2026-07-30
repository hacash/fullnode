use base::{BinaryCodecs, Transaction};
use field::Sign;
use protocol::tx_std::{TransactionType1, TransactionType2, TransactionType3};
use sys::{Account as SysAccount, ToHex};
use wasm_bindgen::prelude::*;

use crate::codec::standard_codecs;
use crate::error::{fault, js_error};

#[derive(Default)]
#[wasm_bindgen(getter_with_clone, inspectable)]
pub struct SignTxParam {
    pub prikey: String,
    pub body: String,
}

#[wasm_bindgen]
impl SignTxParam {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }
}

#[wasm_bindgen(getter_with_clone, inspectable)]
pub struct SignTxResult {
    pub hash: String,
    pub hash_with_fee: String,
    pub body: String,
    pub signature: String,
    pub timestamp: u64,
}

fn build_result(transaction: &dyn Transaction, signature: &Sign) -> SignTxResult {
    SignTxResult {
        hash: transaction.hash().0.to_hex(),
        hash_with_fee: transaction.hash_with_fee().0.to_hex(),
        body: transaction.encode().to_hex(),
        signature: signature.signature.to_hex(),
        timestamp: transaction.timestamp().value(),
    }
}

fn sign_transaction_inner(param: SignTxParam) -> sys::Ret<SignTxResult> {
    let account = SysAccount::create_by(&param.prikey)
        .map_err(|error| fault("private key invalid", error))?;
    let body =
        hex::decode(&param.body).map_err(|error| fault("tx body hex decode failed", error))?;
    let transaction = standard_codecs()?
        .decode_transaction_exact(&body)
        .map_err(|error| fault("tx parse failed", error))?;

    macro_rules! sign_concrete {
        ($transaction_type:ty) => {{
            let source = transaction
                .as_any()
                .downcast_ref::<$transaction_type>()
                .ok_or_else(|| sys::Error::fault("transaction type downcast failed"))?;
            let mut signed = source.clone();
            let signature = signed
                .fill_sign_account(&account)
                .map_err(|error| fault("sign failed", error))?;
            Ok(build_result(&signed, &signature))
        }};
    }

    match transaction.ty() {
        TransactionType1::TYPE => sign_concrete!(TransactionType1),
        TransactionType2::TYPE => sign_concrete!(TransactionType2),
        TransactionType3::TYPE => sign_concrete!(TransactionType3),
        ty => sys::errf!("transaction type {} cannot be signed by the wasm sdk", ty),
    }
}

#[wasm_bindgen]
pub fn sign_transaction(param: SignTxParam) -> Result<SignTxResult, JsValue> {
    sign_transaction_inner(param).map_err(js_error)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base::TransactionBuild;
    use field::{Address, Amount, Encode, Uint1};
    use protocol::action_std::HacToTrs;

    use super::*;

    const LEGACY_BODY: &str = "0200689e96d400e63c33a796b3032ce6b856f68fccf06608d9ed18f401010002000100e63c33a796b3032ce6b856f68fccf06608d9ed18f8010c000a00e63c33a796b3032ce6b856f68fccf06608d9ed180000000000b71b0000010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263c9b607f2bd9e1031536c13741facb78585755c116aa7d10628ebc2adbb4be96493bc1bb8ac6c3e78dee6717b9c4a27280b698efc91097d5900418a59c9d8e7ac30000";

    #[test]
    fn signs_current_standard_transaction_codec() {
        let result = sign_transaction_inner(SignTxParam {
            prikey: "123456".to_owned(),
            body: LEGACY_BODY.to_owned(),
        })
        .unwrap();
        assert_eq!(result.body, LEGACY_BODY);
        assert_eq!(result.signature.len(), 128);
        assert_eq!(result.timestamp, 1_755_223_764);
    }

    #[test]
    fn rejects_trailing_bytes_instead_of_signing_only_a_prefix() {
        let error = sign_transaction_inner(SignTxParam {
            prikey: "123456".to_owned(),
            body: format!("{}00", LEGACY_BODY),
        })
        .err()
        .unwrap();
        assert!(error.to_string().contains("parse length mismatch"));
    }

    #[test]
    fn signs_current_type3_transaction() {
        let main = SysAccount::create_by("123456").unwrap();
        let to = SysAccount::create_by("654321").unwrap();
        let mut transaction = TransactionType3::new_by(
            Address::from(*main.address()),
            Amount::from("1:244").unwrap(),
            1_755_223_764,
        );
        transaction.gas_max = Uint1::from(10);
        transaction
            .push_action(Arc::new(HacToTrs::new(
                Address::from(*to.address()),
                Amount::from("1:244").unwrap(),
            )))
            .unwrap();

        let result = sign_transaction_inner(SignTxParam {
            prikey: "123456".to_owned(),
            body: transaction.encode().to_hex(),
        })
        .unwrap();
        let signed = standard_codecs()
            .unwrap()
            .decode_transaction_exact(&hex::decode(result.body).unwrap())
            .unwrap();
        assert_eq!(signed.ty(), TransactionType3::TYPE);
        signed.verify_signature().unwrap();
    }
}
