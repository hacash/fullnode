use base58check::*;
use libsecp256k1::{ util, SecretKey, PublicKey, Signature, Message };
// use rand::{self, RngCore};


const ADDRESS_SIZE: usize = 21;
const PRIVATE_SIZE: usize = 32;
const PUBLIC_SIZE: usize = 33;


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
            return Ok(())
        }
        // not match
        errf!("Account check failed, need {} but got {}", 
            self.address_readable, Self::to_base58check(addr))
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
    pub fn readable(&self) -> &String {
        &self.address_readable
    }
}




impl Account {

    // create
    
    pub fn create_randomly(randomfill: &dyn Fn(&mut [u8])->Rerr) -> Ret<Account> {
        loop {
            let mut data = [0u8; PRIVATE_SIZE];
            /*if let Err(e) = getrandom::fill(&mut data) {
                return Err(e.to_string())
            }*/
            randomfill(&mut data)?;
            // println!("{:?}", data)
            if data[0] < 255 {
                return Account::create_by_secret_key_value(data)
            }
        }
    }
    

    pub fn create_by(pass: &str) -> Ret<Account> {
        // is private key
        if pass.len() == PRIVATE_SIZE * 2 {
            if let Ok(bts) = hex::decode(pass) {
                return Account::create_by_secret_key_value(bts.try_into().unwrap())
            }
        }
        // is passward
        return Account::create_by_password(pass)
    }

    pub fn create_by_password(pass: &str) -> Ret<Account> {
        let dt = sha2(pass);
        Account::create_by_secret_key_value(dt)
    }

    pub fn create_by_secret_key_value(key32: [u8; PRIVATE_SIZE]) -> Ret<Account> {
        let kkk = key32.to_vec();
        if kkk[0] == 255 && kkk[1] == 255 && kkk[2] == 255 && kkk[3] == 255 {
            return Err("not support secret_key, change one and try again.".to_string());
        }
        let pk: [u8; util::SECRET_KEY_SIZE] = kkk.try_into().unwrap();
        match SecretKey::parse(&pk) {
            Err(e) => Err(e.to_string()),
            Ok(sk) => Ok(Account::create_by_secret_key(&sk)),
        }
    }

    fn create_by_secret_key(seckey: &SecretKey) -> Account {
        let pubkey = PublicKey::from_secret_key(seckey);
        let address = Account::get_address_by_public_key( pubkey.serialize_compressed() );
        let addrshow = Account::to_readable(&address);
        Account {
            secret_key: seckey.clone(),
            public_key: pubkey,
            address: address,
            address_readable: addrshow,
        }
    }


    pub fn get_address_by_public_key(pubkey: [u8; PUBLIC_SIZE]) -> [u8; ADDRESS_SIZE] {
        // serialize_compressed
        let dt = sha2(pubkey);
        let dt = ripemd160(dt);
        let version = 0;
        let mut addr = [version; ADDRESS_SIZE];
        addr[1..].copy_from_slice(&dt[..]);
        addr
    }


    pub fn to_readable(addr: &[u8; ADDRESS_SIZE]) -> String {
        let version = addr[0];
        addr[1..].to_base58check(version)
    }

    pub fn to_base58check(s: &[u8]) -> String {
        let v = maybe!(s.len() > 0, s[0], 0);
        let b = maybe!(s.len() > 1, &s[1..], &s[..]);
        b.to_base58check(v)
    }

}



// signature
impl Account {

    // return Signature
    pub fn do_sign(&self, msg: &[u8; 32]) -> [u8; 64] {
        let msg = Message::parse(msg);
        let (s, _r) = libsecp256k1::sign(&msg, &self.secret_key);
        s.serialize()
    }

    pub fn verify_signature(msg: &[u8; 32], publickey: &[u8; 33], signature: &[u8; 64]) -> bool {
        if let Ok(pubkey) = PublicKey::parse_compressed(publickey) {
            if let Ok(sigobj) = Signature::parse_standard(signature) {
                return libsecp256k1::verify(
                    &Message::parse(msg),
                    &sigobj,
                    &pubkey,
                )
            }
        }
        false
    }

}
        

