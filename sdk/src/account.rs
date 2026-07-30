use field::Address;
use sys::Account as SysAccount;
use wasm_bindgen::prelude::*;

use crate::error::js_error;

#[wasm_bindgen(getter_with_clone, inspectable)]
pub struct Account {
    pub prikey: String,
    pub pubkey: String,
    pub address: String,
    pub address_hex: String,
}

fn create_account_inner(pass: &str) -> sys::Ret<Account> {
    SysAccount::create_by(pass).map(|account| Account {
        prikey: hex::encode(account.secret_key().serialize()),
        pubkey: hex::encode(account.public_key().serialize_compressed()),
        address: account.readable().to_owned(),
        address_hex: hex::encode(account.address()),
    })
}

/// `pass` is either a 32-byte private key in hex or a password, matching the
/// historical JS SDK behavior.
#[wasm_bindgen]
pub fn create_account(pass: &str) -> Result<Account, JsValue> {
    create_account_inner(pass).map_err(js_error)
}

#[wasm_bindgen(getter_with_clone, inspectable)]
pub struct VerifyAddressResult {
    pub ok: bool,
    pub error: String,
}

#[wasm_bindgen]
pub fn verify_address(address: &str) -> VerifyAddressResult {
    match Address::from_readable(address) {
        Ok(_) => VerifyAddressResult {
            ok: true,
            error: String::new(),
        },
        Err(error) => VerifyAddressResult {
            ok: false,
            error: error.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_and_equivalent_private_key_create_same_account() {
        let by_password = create_account_inner("123456").unwrap();
        let by_key = create_account_inner(
            "8d969eef6ecad3c29a3a629280e686cf0c3f5d5a86aff3ca12020c923adc6c92",
        )
        .unwrap();
        assert_eq!(by_password.prikey, by_key.prikey);
        assert_eq!(by_password.pubkey, by_key.pubkey);
        assert_eq!(by_password.address, by_key.address);
    }

    #[test]
    fn address_validation_checks_checksum_and_supported_version() {
        assert!(verify_address("1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").ok);
        assert!(!verify_address("2MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9").ok);
    }
}
