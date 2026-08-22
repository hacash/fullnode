//! Account services (Unified SDK 2.0, doc 14 §5). Private keys never enter the
//! SDK: addresses derive from public keys only; password→key derivation lives in the vault.

use field::Address;

use crate::error::{SdkError, SdkErrorCode};

#[derive(Debug, Clone, PartialEq)]
pub struct VerifyAddressResult {
    pub ok: bool,
    pub error: Option<String>,
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AddressFromPublicKeyResult {
    pub address: String,
    pub version: u8,
}

/// `account.verify_signature`: verify a 64-byte secp256k1 signature over a
/// 32-byte digest with a 33-byte compressed public key, and return the derived
/// signer address. The raw primitive for exchange-side API-signature checks
/// (message.verify is the request/proof flow; this one takes plain inputs).
#[derive(Debug, Clone, PartialEq)]
pub struct VerifySignatureResult {
    pub ok: bool,
    pub address: Option<String>,
    pub error: Option<String>,
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

/// `account.address_from_public_key`: derive the Hacash address from a 33-byte
/// compressed public key (hex); no secret input, the point must be valid secp256k1.
pub fn address_from_public_key(
    public_key_hex: &str,
) -> Result<AddressFromPublicKeyResult, SdkError> {
    let public_key: [u8; 33] = crate::inspect::decode_hex_fixed(
        public_key_hex,
        SdkErrorCode::InvalidPublicKey,
        "public key must be 33-byte compressed hex",
    )?;
    if !sys::Account::compressed_public_key_valid(&public_key) {
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

/// `account.verify_signature`: malformed inputs are caller bugs (Err), a
/// well-formed key with a wrong signature is a factual `{ok:false}` result.
pub fn verify_signature(
    public_key_hex: &str,
    digest_hex: &str,
    signature_hex: &str,
) -> Result<VerifySignatureResult, SdkError> {
    let public_key: [u8; 33] = crate::inspect::decode_hex_fixed(
        public_key_hex,
        SdkErrorCode::InvalidPublicKey,
        "public key must be 33-byte compressed hex",
    )?;
    if !sys::Account::compressed_public_key_valid(&public_key) {
        return Err(SdkError::new(
            SdkErrorCode::InvalidPublicKey,
            "public key is not a valid secp256k1 compressed point",
        ));
    }
    let digest: [u8; 32] = crate::inspect::decode_hex_fixed(
        digest_hex,
        SdkErrorCode::ParseFailed,
        "digest must be 32-byte hex",
    )?;
    let signature: [u8; 64] = crate::inspect::decode_hex_fixed(
        signature_hex,
        SdkErrorCode::BadSignature,
        "signature must be 64-byte hex",
    )?;
    let ok = sys::Account::verify_signature(&digest, &public_key, &signature);
    let address = Address::from(sys::Account::get_address_by_public_key(public_key));
    Ok(VerifySignatureResult {
        ok,
        address: if ok { Some(address.to_readable()) } else { None },
        error: if ok {
            None
        } else {
            Some("signature verification failed".to_owned())
        },
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

    #[test]
    fn verifies_a_real_signature() {
        let account = sys::Account::create_by("123456").unwrap();
        let digest = sys::calculate_hash(b"exchange-api-challenge");
        let signature = account.do_sign(&digest);
        let result = verify_signature(
            &hex::encode(account.public_key().serialize_compressed()),
            &hex::encode(digest),
            &hex::encode(signature),
        )
        .unwrap();
        assert!(result.ok);
        assert_eq!(result.address.unwrap(), account.readable());
        assert!(result.error.is_none());
    }

    #[test]
    fn rejects_wrong_signature_as_a_fact() {
        let account = sys::Account::create_by("123456").unwrap();
        let other = sys::Account::create_by("other-key-9").unwrap();
        let digest = sys::calculate_hash(b"challenge");
        let signature = other.do_sign(&digest);
        let result = verify_signature(
            &hex::encode(account.public_key().serialize_compressed()),
            &hex::encode(digest),
            &hex::encode(signature),
        )
        .unwrap();
        assert!(!result.ok);
        assert!(result.address.is_none());
        assert!(result.error.is_some());
    }

    #[test]
    fn rejects_malformed_inputs() {
        let account = sys::Account::create_by("123456").unwrap();
        let digest = sys::calculate_hash(b"challenge");
        let signature = account.do_sign(&digest);
        assert_eq!(
            verify_signature(
                &hex::encode(account.public_key().serialize_compressed()),
                "not-hex",
                &hex::encode(signature),
            )
            .unwrap_err()
            .code,
            "parse_failed"
        );
        assert_eq!(
            verify_signature(
                &hex::encode(account.public_key().serialize_compressed()),
                &hex::encode(digest),
                "aabb",
            )
            .unwrap_err()
            .code,
            "bad_signature"
        );
    }
}
