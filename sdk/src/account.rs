//! Account services (Unified SDK 2.0, doc 14 §5). Private keys never enter
//! the SDK: `address_from_public_key` derives an address from a public key
//! only; password→key derivation lives in the wallet vault.

use field::Address;
use serde::{Deserialize, Serialize};

use crate::error::{SdkError, SdkErrorCode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyAddressResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressFromPublicKeyResult {
    pub address: String,
    pub version: u8,
}

/// `account.verify_address`: parse and canonicalize a readable address.
pub fn verify_address(raw: &str) -> VerifyAddressResult {
    match Address::from_readable(raw) {
        Ok(address) => VerifyAddressResult {
            ok: true,
            error: None,
            address: Some(address.to_readable()),
        },
        Err(error) => VerifyAddressResult {
            ok: false,
            error: Some(error.to_string()),
            address: None,
        },
    }
}

/// `account.address_from_public_key`: derive the Hacash address from a
/// 33-byte compressed public key (hex). No secret input. The point must be a
/// valid secp256k1 curve point.
pub fn address_from_public_key(public_key_hex: &str) -> Result<AddressFromPublicKeyResult, SdkError> {
    let public_key: [u8; 33] = hex::decode(public_key_hex.trim_start_matches("0x").trim_start_matches("0X"))
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| {
            SdkError::new(
                SdkErrorCode::InvalidPublicKey,
                "public key must be 33-byte compressed hex",
            )
        })?;
    if libsecp256k1::PublicKey::parse_compressed(&public_key).is_err() {
        return Err(SdkError::new(
            SdkErrorCode::InvalidPublicKey,
            "public key is not a valid secp256k1 compressed point",
        ));
    }
    let address = Address::from(sys::Account::get_address_by_public_key(public_key));
    Ok(AddressFromPublicKeyResult {
        address: address.to_readable(),
        version: address.version(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_address_from_public_key() {
        let account = sys::Account::create_by("123456").unwrap();
        let pubkey = hex::encode(account.public_key().serialize_compressed());
        let result = address_from_public_key(&pubkey).unwrap();
        assert_eq!(result.address, account.readable());
        assert_eq!(result.version, 0);
    }

    #[test]
    fn rejects_bad_public_keys() {
        assert_eq!(
            address_from_public_key("aabb").unwrap_err().code,
            "invalid_public_key"
        );
        assert_eq!(
            address_from_public_key("00".repeat(33).as_str())
                .unwrap_err()
                .code,
            "invalid_public_key"
        );
    }
}
