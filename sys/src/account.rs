use base58check::ToBase58Check;
#[cfg(not(feature = "secp-static-context"))]
use libsecp256k1::curve::{ECMultContext, ECMultGenContext};
use libsecp256k1::{Message, PublicKey, SecretKey, Signature, util};
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};
#[cfg(not(feature = "secp-static-context"))]
use std::sync::OnceLock;

use crate::{Rerr, Ret, errf};

const ADDRESS_SIZE: usize = 21;
const PRIVATE_SIZE: usize = 32;
const PUBLIC_SIZE: usize = 33;

// Two secp256k1 context strategies (see `secp-static-context`): fullnode embeds
// precomputed tables (free sign/verify); SDK/wasm computes the ~1MB ecmult tables once on first use.
#[cfg(feature = "secp-static-context")]
fn pubkey_from_secret_key(seckey: &SecretKey) -> PublicKey {
    PublicKey::from_secret_key(seckey)
}

#[cfg(not(feature = "secp-static-context"))]
fn pubkey_from_secret_key(seckey: &SecretKey) -> PublicKey {
    PublicKey::from_secret_key_with_context(seckey, ecmult_gen_ctx())
}

#[cfg(feature = "secp-static-context")]
fn sign_impl(msg: &Message, seckey: &SecretKey) -> (Signature, libsecp256k1::RecoveryId) {
    libsecp256k1::sign(msg, seckey)
}

#[cfg(not(feature = "secp-static-context"))]
fn sign_impl(msg: &Message, seckey: &SecretKey) -> (Signature, libsecp256k1::RecoveryId) {
    libsecp256k1::sign_with_context(msg, seckey, ecmult_gen_ctx())
}

#[cfg(feature = "secp-static-context")]
fn verify_impl(msg: &Message, signature: &Signature, pubkey: &PublicKey) -> bool {
    libsecp256k1::verify(msg, signature, pubkey)
}

#[cfg(not(feature = "secp-static-context"))]
fn verify_impl(msg: &Message, signature: &Signature, pubkey: &PublicKey) -> bool {
    libsecp256k1::verify_with_context(msg, signature, pubkey, ecmult_ctx())
}

#[cfg(not(feature = "secp-static-context"))]
fn ecmult_ctx() -> &'static ECMultContext {
    static CTX: OnceLock<Box<ECMultContext>> = OnceLock::new();
    &**CTX.get_or_init(ECMultContext::new_boxed)
}

#[cfg(not(feature = "secp-static-context"))]
fn ecmult_gen_ctx() -> &'static ECMultGenContext {
    static CTX: OnceLock<Box<ECMultGenContext>> = OnceLock::new();
    &**CTX.get_or_init(ECMultGenContext::new_boxed)
}

#[derive(Clone, PartialEq)]
pub struct Account {
    secret_key: SecretKey,
    public_key: PublicKey,
    address: [u8; ADDRESS_SIZE],
    address_readable: String,
}

impl Account {
    pub fn check_addr(&self, addr: &[u8]) -> Rerr {
        if self.address == *addr {
            return Ok(());
        }
        errf!(
            "Account check failed: expected {} but got {}",
            self.address_readable,
            Self::to_base58check(addr)
        )
    }

    pub fn secret_key(&self) -> &SecretKey {
        &self.secret_key
    }

    pub fn public_key(&self) -> &PublicKey {
        &self.public_key
    }

    pub fn address(&self) -> &[u8; ADDRESS_SIZE] {
        &self.address
    }

    pub fn readable(&self) -> &str {
        &self.address_readable
    }

    pub fn create_randomly(randomfill: &dyn Fn(&mut [u8]) -> Rerr) -> Ret<Account> {
        loop {
            let mut data = [0u8; PRIVATE_SIZE];
            randomfill(&mut data)?;
            match Account::create_by_secret_key_value(data) {
                Ok(acc) => return Ok(acc),
                Err(_) => continue,
            }
        }
    }

    pub fn create_by(pass: &str) -> Ret<Account> {
        if pass.len() == PRIVATE_SIZE * 2 {
            if let Ok(bts) = hex::decode(pass) {
                if bts.len() == PRIVATE_SIZE {
                    let mut key = [0u8; PRIVATE_SIZE];
                    key.copy_from_slice(&bts);
                    return Account::create_by_secret_key_value(key);
                }
            }
        }
        Account::create_by_password(pass)
    }

    pub fn create_by_password(pass: &str) -> Ret<Account> {
        Account::create_by_secret_key_value(sha2_256(pass.as_bytes()))
    }

    pub fn create_by_secret_key_value(key32: [u8; PRIVATE_SIZE]) -> Ret<Account> {
        if key32[0] == 255 && key32[1] == 255 && key32[2] == 255 && key32[3] == 255 {
            return errf!("secret_key not supported; try a different one");
        }
        let pk: [u8; util::SECRET_KEY_SIZE] = key32;
        let sk = SecretKey::parse(&pk).map_err(|e| crate::Error::fault(e.to_string()))?;
        Ok(Account::create_by_secret_key(&sk))
    }

    fn create_by_secret_key(seckey: &SecretKey) -> Account {
        let pubkey = pubkey_from_secret_key(seckey);
        let address = Account::get_address_by_public_key(pubkey.serialize_compressed());
        let address_readable = Account::to_readable(&address);
        Account {
            secret_key: *seckey,
            public_key: pubkey,
            address,
            address_readable,
        }
    }

    pub fn get_address_by_public_key(pubkey: [u8; PUBLIC_SIZE]) -> [u8; ADDRESS_SIZE] {
        let dt = sha2_256(&pubkey);
        let dt = ripemd160(&dt);
        let version = 0;
        let mut addr = [version; ADDRESS_SIZE];
        addr[1..].copy_from_slice(&dt);
        addr
    }

    /// Whether `pubkey` is a valid secp256k1 compressed point (same check
    /// `verify_signature` runs before hashing; address derivation stays hash-only).
    pub fn compressed_public_key_valid(pubkey: &[u8; PUBLIC_SIZE]) -> bool {
        PublicKey::parse_compressed(pubkey).is_ok()
    }

    pub fn to_readable(addr: &[u8; ADDRESS_SIZE]) -> String {
        let version = addr[0];
        addr[1..].to_base58check(version)
    }

    pub fn to_base58check(s: &[u8]) -> String {
        let v = s.first().copied().unwrap_or(0);
        let b = if s.is_empty() { &[][..] } else { &s[1..] };
        b.to_base58check(v)
    }

    pub fn do_sign(&self, msg: &[u8; 32]) -> [u8; 64] {
        let msg = Message::parse(msg);
        let (s, _r) = sign_impl(&msg, &self.secret_key);
        s.serialize()
    }

    pub fn verify_signature(msg: &[u8; 32], publickey: &[u8; 33], signature: &[u8; 64]) -> bool {
        if let Ok(pubkey) = PublicKey::parse_compressed(publickey) {
            if let Ok(sigobj) = Signature::parse_standard(signature) {
                return verify_impl(&Message::parse(msg), &sigobj, &pubkey);
            }
        }
        false
    }
}

fn sha2_256(data: &[u8]) -> [u8; 32] {
    let out = Sha256::digest(data);
    let mut buf = [0u8; 32];
    buf.copy_from_slice(&out);
    buf
}

fn ripemd160(data: &[u8]) -> [u8; 20] {
    let out = Ripemd160::digest(data);
    let mut buf = [0u8; 20];
    buf.copy_from_slice(&out);
    buf
}
