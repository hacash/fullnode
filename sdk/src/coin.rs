use std::sync::Arc;

use base::{ActionRef, Transaction, TransactionBuild};
use field::{Address, Amount, DiamondName, DiamondNameListMax200, Encode, Satoshi};
use protocol::action_std::{
    DiaFromToTrs, DiaSingleTrs, DiaToTrs, HacFromToTrs, HacToTrs, SatFromToTrs, SatToTrs,
};
use protocol::tx_std::TransactionType2;
use sys::{Account as SysAccount, ToHex};
use wasm_bindgen::prelude::*;

use crate::error::{fault, js_error};

#[derive(Default)]
#[wasm_bindgen(getter_with_clone, inspectable)]
pub struct CoinTransferParam {
    pub main_prikey: String,
    pub from_prikey: String,
    pub fee: String,
    pub to_address: String,
    pub timestamp: u64,
    pub hacash: String,
    pub satoshi: u64,
    pub diamonds: String,
    /// Reserved by the historical JS ABI. A Type2 transfer does not encode a
    /// chain id, so changing its meaning here would change transaction bytes.
    pub chain_id: u64,
}

#[wasm_bindgen]
impl CoinTransferParam {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self::default()
    }
}

#[wasm_bindgen(getter_with_clone, inspectable)]
pub struct CoinTransferResult {
    pub hash: String,
    pub hash_with_fee: String,
    pub body: String,
    pub timestamp: u64,
}

fn parse_account(stuff: &str) -> sys::Ret<SysAccount> {
    SysAccount::create_by(stuff).map_err(|error| fault("private key invalid", error))
}

fn parse_amount(stuff: &str) -> sys::Ret<Amount> {
    Amount::from(stuff).map_err(|error| fault("amount invalid", error))
}

fn parse_address(stuff: &str) -> sys::Ret<Address> {
    Address::from_readable(stuff).map_err(|error| fault("address invalid", error))
}

fn push_action(transaction: &mut TransactionType2, action: ActionRef, name: &str) -> sys::Rerr {
    transaction
        .push_action(action)
        .map_err(|error| fault(&format!("push {} action failed", name), error))
}

fn create_coin_transfer_inner(param: CoinTransferParam) -> sys::Ret<CoinTransferResult> {
    let main = parse_account(&param.main_prikey)?;
    let main_address = Address::from(*main.address());
    let from = if param.from_prikey.is_empty() {
        main.clone()
    } else {
        parse_account(&param.from_prikey)?
    };
    let from_address = Address::from(*from.address());
    let other_from = from != main;
    let fee = parse_amount(&param.fee)?;
    let to_address = parse_address(&param.to_address)?;
    let timestamp = if param.timestamp == 0 {
        sys::curtimes()
    } else {
        param.timestamp
    };

    let mut transaction = TransactionType2::new_by(main_address, fee, timestamp);

    if !param.hacash.is_empty() {
        let amount = Amount::from(&param.hacash)
            .map_err(|error| fault(&format!("hacash amount {} invalid", param.hacash), error))?;
        let action: ActionRef = if other_from {
            Arc::new(HacFromToTrs::new(from_address, to_address, amount))
        } else {
            Arc::new(HacToTrs::new(to_address, amount))
        };
        push_action(&mut transaction, action, "hac transfer")?;
    }

    if param.satoshi > 0 {
        let satoshi = Satoshi::from(param.satoshi);
        let action: ActionRef = if other_from {
            Arc::new(SatFromToTrs::new(from_address, to_address, satoshi))
        } else {
            Arc::new(SatToTrs::new(to_address, satoshi))
        };
        push_action(&mut transaction, action, "sat transfer")?;
    }

    if param.diamonds.len() >= DiamondName::SIZE {
        let diamonds = DiamondNameListMax200::from_readable(&param.diamonds)
            .map_err(|error| fault("diamonds invalid", error))?;
        let action: ActionRef = if other_from {
            Arc::new(DiaFromToTrs::new(from_address, to_address, diamonds))
        } else if diamonds.length() == 1 {
            Arc::new(DiaSingleTrs::new(diamonds.as_list()[0], to_address))
        } else {
            Arc::new(DiaToTrs::new(to_address, diamonds))
        };
        push_action(&mut transaction, action, "diamond transfer")?;
    }

    transaction
        .fill_sign_account(&main)
        .map_err(|error| fault("fill main sign failed", error))?;
    if other_from {
        transaction
            .fill_sign_account(&from)
            .map_err(|error| fault("fill from sign failed", error))?;
    }

    Ok(CoinTransferResult {
        hash: transaction.hash().0.to_hex(),
        hash_with_fee: transaction.hash_with_fee().0.to_hex(),
        body: transaction.encode().to_hex(),
        timestamp,
    })
}

#[wasm_bindgen]
pub fn create_coin_transfer(param: CoinTransferParam) -> Result<CoinTransferResult, JsValue> {
    create_coin_transfer_inner(param).map_err(js_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_legacy_type2_transfer_bytes() {
        let result = create_coin_transfer_inner(CoinTransferParam {
            main_prikey: "123456".to_owned(),
            from_prikey: String::new(),
            fee: "1:244".to_owned(),
            to_address: "1MzNY1oA3kfgYi75zquj3SRUPYztzXHzK9".to_owned(),
            timestamp: 1_755_223_764,
            hacash: "12.0".to_owned(),
            satoshi: 12_000_000,
            diamonds: String::new(),
            chain_id: 0,
        })
        .unwrap();

        assert_eq!(
            result.hash,
            "0b6f0b86427acc0834805a517f7fb943a38ae98d0deb52beeaa86f82679323c2"
        );
        assert_eq!(
            result.hash_with_fee,
            "0beace8e96696686e068e8e2b97cd23da276ef42294ef2eb2e62eaf064e3715f"
        );
        assert_eq!(
            result.body,
            "0200689e96d400e63c33a796b3032ce6b856f68fccf06608d9ed18f401010002000100e63c33a796b3032ce6b856f68fccf06608d9ed18f8010c000a00e63c33a796b3032ce6b856f68fccf06608d9ed180000000000b71b0000010231745adae24044ff09c3541537160abb8d5d720275bbaeed0b3d035b1e8b263c9b607f2bd9e1031536c13741facb78585755c116aa7d10628ebc2adbb4be96493bc1bb8ac6c3e78dee6717b9c4a27280b698efc91097d5900418a59c9d8e7ac30000"
        );
    }
}
