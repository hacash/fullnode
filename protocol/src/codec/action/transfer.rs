//! Hac / Sat / Asset / Diamond transfer actions.

use base::{ActScope, AddrOrPtr, TransferLike, TransferPayload};
use field::{
    Address, Amount, AssetAmt, DiamondName, DiamondNameListMax200, Encode, Satoshi, Uint2,
};
use sys::Ret;

/// Readable rendering of a wire destination / source (address or pointer).
pub(super) fn addr_or_ptr_readable(ptr: &AddrOrPtr) -> String {
    match ptr {
        AddrOrPtr::Addr(addr) => addr.to_readable(),
        AddrOrPtr::Ptr(index) => format!("<address pointer {}>", index),
    }
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct HacToTrs {
    pub kind: Uint2,
    pub to: AddrOrPtr,
    pub hacash: Amount,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct HacFromTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub hacash: Amount,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct HacFromToTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub to: AddrOrPtr,
    pub hacash: Amount,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct SatToTrs {
    pub kind: Uint2,
    pub to: AddrOrPtr,
    pub satoshi: Satoshi,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct SatFromTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub satoshi: Satoshi,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct SatFromToTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub to: AddrOrPtr,
    pub satoshi: Satoshi,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct AssetToTrs {
    pub kind: Uint2,
    pub to: AddrOrPtr,
    pub asset: AssetAmt,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct AssetFromTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub asset: AssetAmt,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full")]
pub struct AssetFromToTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub to: AddrOrPtr,
    pub asset: AssetAmt,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full", validate = "Self::validate_codec")]
pub struct DiaSingleTrs {
    pub kind: Uint2,
    pub diamond: DiamondName,
    pub to: AddrOrPtr,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full", validate = "Self::validate_codec")]
pub struct DiaFromToTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub to: AddrOrPtr,
    pub diamonds: DiamondNameListMax200,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full", validate = "Self::validate_codec")]
pub struct DiaToTrs {
    pub kind: Uint2,
    pub to: AddrOrPtr,
    pub diamonds: DiamondNameListMax200,
}

#[derive(Debug, Clone, base::ActionCodec)]
#[action_codec(audit = "full", validate = "Self::validate_codec")]
pub struct DiaFromTrs {
    pub kind: Uint2,
    pub from: AddrOrPtr,
    pub diamonds: DiamondNameListMax200,
}

impl HacToTrs {
    pub const KIND: u16 = 1;

    pub fn new(to: Address, amount: Amount) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            to: AddrOrPtr::Addr(to),
            hacash: amount,
        }
    }
}

impl HacFromTrs {
    pub const KIND: u16 = 13;

    pub fn new(from: Address, amount: Amount) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from: AddrOrPtr::Addr(from),
            hacash: amount,
        }
    }
}

impl HacFromToTrs {
    pub const KIND: u16 = 14;

    pub fn new(from: Address, to: Address, amount: Amount) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from: AddrOrPtr::Addr(from),
            to: AddrOrPtr::Addr(to),
            hacash: amount,
        }
    }
}

impl SatToTrs {
    pub const KIND: u16 = 10;

    pub fn new(to: Address, satoshi: Satoshi) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            to: AddrOrPtr::Addr(to),
            satoshi,
        }
    }
}

impl SatFromTrs {
    pub const KIND: u16 = 11;

    pub fn new(from: Address, satoshi: Satoshi) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from: AddrOrPtr::Addr(from),
            satoshi,
        }
    }
}

impl SatFromToTrs {
    pub const KIND: u16 = 12;

    pub fn new(from: Address, to: Address, satoshi: Satoshi) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from: AddrOrPtr::Addr(from),
            to: AddrOrPtr::Addr(to),
            satoshi,
        }
    }
}

impl AssetToTrs {
    pub const KIND: u16 = 17;

    pub fn new(to: Address, asset: AssetAmt) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            to: AddrOrPtr::Addr(to),
            asset,
        }
    }
}

impl AssetFromTrs {
    pub const KIND: u16 = 18;

    pub fn new(from: Address, asset: AssetAmt) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from: AddrOrPtr::Addr(from),
            asset,
        }
    }
}

impl AssetFromToTrs {
    pub const KIND: u16 = 19;

    pub fn new(from: Address, to: Address, asset: AssetAmt) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from: AddrOrPtr::Addr(from),
            to: AddrOrPtr::Addr(to),
            asset,
        }
    }
}

impl DiaSingleTrs {
    pub const KIND: u16 = 5;

    pub fn new(diamond: DiamondName, to: Address) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            diamond,
            to: AddrOrPtr::Addr(to),
        }
    }

    fn validate_codec(&self) -> Ret<()> {
        DiamondName::check_bytes(self.diamond.as_ref())
    }
}

impl DiaFromToTrs {
    pub const KIND: u16 = 6;

    pub fn new(from: Address, to: Address, diamonds: DiamondNameListMax200) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from: AddrOrPtr::Addr(from),
            to: AddrOrPtr::Addr(to),
            diamonds,
        }
    }

    fn validate_codec(&self) -> Ret<()> {
        self.diamonds.check().map(|_| ())
    }
}

impl DiaToTrs {
    pub const KIND: u16 = 7;

    pub fn new(to: Address, diamonds: DiamondNameListMax200) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            to: AddrOrPtr::Addr(to),
            diamonds,
        }
    }

    fn validate_codec(&self) -> Ret<()> {
        self.diamonds.check().map(|_| ())
    }
}

impl DiaFromTrs {
    pub const KIND: u16 = 8;

    pub fn new(from: Address, diamonds: DiamondNameListMax200) -> Self {
        Self {
            kind: Uint2::from(Self::KIND),
            from: AddrOrPtr::Addr(from),
            diamonds,
        }
    }

    fn validate_codec(&self) -> Ret<()> {
        self.diamonds.check().map(|_| ())
    }
}

impl TransferLike for HacToTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        &self.hacash
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Hac {
            amount: self.hacash.encode(),
        }
    }
}

impl TransferLike for HacFromTrs {
    fn transfer_to(&self) -> Address {
        Address::default()
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        None
    }
    fn transfer_amount(&self) -> &Amount {
        &self.hacash
    }
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        Some(self.from.clone())
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Hac {
            amount: self.hacash.encode(),
        }
    }
}

impl TransferLike for HacFromToTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        &self.hacash
    }
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        Some(self.from.clone())
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Hac {
            amount: self.hacash.encode(),
        }
    }
}

impl TransferLike for SatToTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Sat {
            satoshi: self.satoshi.uint(),
        }
    }
}

impl TransferLike for SatFromTrs {
    fn transfer_to(&self) -> Address {
        Address::default()
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        None
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        Some(self.from.clone())
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Sat {
            satoshi: self.satoshi.uint(),
        }
    }
}

impl TransferLike for SatFromToTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        Some(self.from.clone())
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Sat {
            satoshi: self.satoshi.uint(),
        }
    }
}

base::impl_action_facts! {
    HacToTrs {
        name: "transfer_hac_to",
        scope: ActScope::CALL,
        min_tx_type: 1,
        extra9: |_: &HacToTrs| false,
        req_sign: |_: &HacToTrs| vec![],
        as_transfer_like: self,
        description: |this: &HacToTrs| format!("Transfer {} HAC to {}", this.hacash.to_unit_string("HAC"), addr_or_ptr_readable(&this.to)),

    }
}

base::impl_action_facts! {
    HacFromTrs {
        name: "transfer_hac_from",
        scope: ActScope::CALL,
        min_tx_type: 1,
        extra9: |_: &HacFromTrs| false,
        req_sign: |this: &HacFromTrs| vec![this.from.clone()],
        as_transfer_like: self,
        description: |this: &HacFromTrs| format!("Transfer {} HAC from {}", this.hacash.to_unit_string("HAC"), addr_or_ptr_readable(&this.from)),

    }
}

base::impl_action_facts! {
    HacFromToTrs {
        name: "transfer_hac_from_to",
        scope: ActScope::CALL,
        min_tx_type: 1,
        extra9: |_: &HacFromToTrs| false,
        req_sign: |this: &HacFromToTrs| vec![this.from.clone()],
        as_transfer_like: self,
        description: |this: &HacFromToTrs| format!("Transfer {} HAC from {} to {}", this.hacash.to_unit_string("HAC"), addr_or_ptr_readable(&this.from), addr_or_ptr_readable(&this.to)),

    }
}

base::impl_action_facts! {
    SatToTrs {
        name: "transfer_sat_to",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &SatToTrs| false,
        req_sign: |_: &SatToTrs| vec![],
        as_transfer_like: self,
        description: |this: &SatToTrs| format!("Transfer {} SAT to {}", this.satoshi.uint(), addr_or_ptr_readable(&this.to)),

    }
}

base::impl_action_facts! {
    SatFromTrs {
        name: "transfer_sat_from",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &SatFromTrs| false,
        req_sign: |this: &SatFromTrs| vec![this.from.clone()],
        as_transfer_like: self,
        description: |this: &SatFromTrs| format!("Transfer {} SAT from {}", this.satoshi.uint(), addr_or_ptr_readable(&this.from)),

    }
}

base::impl_action_facts! {
    SatFromToTrs {
        name: "transfer_sat_from_to",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &SatFromToTrs| false,
        req_sign: |this: &SatFromToTrs| vec![this.from.clone()],
        as_transfer_like: self,
        description: |this: &SatFromToTrs| format!("Transfer {} SAT from {} to {}", this.satoshi.uint(), addr_or_ptr_readable(&this.from), addr_or_ptr_readable(&this.to)),

    }
}

impl TransferLike for AssetToTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Asset {
            serial: self.asset.serial.uint(),
            amount: self.asset.amount.uint(),
        }
    }
}

impl TransferLike for AssetFromTrs {
    fn transfer_to(&self) -> Address {
        Address::default()
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        None
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        Some(self.from.clone())
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Asset {
            serial: self.asset.serial.uint(),
            amount: self.asset.amount.uint(),
        }
    }
}

impl TransferLike for AssetFromToTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        Some(self.from.clone())
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Asset {
            serial: self.asset.serial.uint(),
            amount: self.asset.amount.uint(),
        }
    }
}

base::impl_action_facts! {
    AssetToTrs {
        name: "transfer_asset_to",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &AssetToTrs| true,
        req_sign: |_: &AssetToTrs| vec![],
        as_transfer_like: self,
        description: |this: &AssetToTrs| format!("Transfer {{{}:{}}} to {}", this.asset.serial.uint(), this.asset.amount.uint(), addr_or_ptr_readable(&this.to)),

    }
}

base::impl_action_facts! {
    AssetFromTrs {
        name: "transfer_asset_from",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &AssetFromTrs| true,
        req_sign: |this: &AssetFromTrs| vec![this.from.clone()],
        as_transfer_like: self,
        description: |this: &AssetFromTrs| format!("Transfer {{{}:{}}} from {}", this.asset.serial.uint(), this.asset.amount.uint(), addr_or_ptr_readable(&this.from)),

    }
}

base::impl_action_facts! {
    AssetFromToTrs {
        name: "transfer_asset_from_to",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &AssetFromToTrs| true,
        req_sign: |this: &AssetFromToTrs| vec![this.from.clone()],
        as_transfer_like: self,
        description: |this: &AssetFromToTrs| format!("Transfer {{{}:{}}} from {} to {}", this.asset.serial.uint(), this.asset.amount.uint(), addr_or_ptr_readable(&this.from), addr_or_ptr_readable(&this.to)),

    }
}

fn diamond_names_payload(diamonds: &DiamondNameListMax200) -> Vec<u8> {
    let encoded = diamonds.encode();
    encoded.get(1..).unwrap_or_default().to_vec()
}

impl TransferLike for DiaSingleTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Hacd {
            count: 1,
            names: self.diamond.to_vec(),
        }
    }
}

impl TransferLike for DiaToTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Hacd {
            count: self.diamonds.length() as u32,
            names: diamond_names_payload(&self.diamonds),
        }
    }
}

impl TransferLike for DiaFromTrs {
    fn transfer_to(&self) -> Address {
        Address::default()
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        None
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        Some(self.from.clone())
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Hacd {
            count: self.diamonds.length() as u32,
            names: diamond_names_payload(&self.diamonds),
        }
    }
}

impl TransferLike for DiaFromToTrs {
    fn transfer_to(&self) -> Address {
        match self.to {
            AddrOrPtr::Addr(addr) => addr,
            AddrOrPtr::Ptr(_) => Address::default(),
        }
    }
    fn transfer_to_ptr(&self) -> Option<AddrOrPtr> {
        Some(self.to.clone())
    }
    fn transfer_amount(&self) -> &Amount {
        Amount::zero_ref()
    }
    fn transfer_from(&self) -> Option<AddrOrPtr> {
        Some(self.from.clone())
    }
    fn transfer_payload(&self) -> TransferPayload {
        TransferPayload::Hacd {
            count: self.diamonds.length() as u32,
            names: diamond_names_payload(&self.diamonds),
        }
    }
}

base::impl_action_facts! {
    DiaSingleTrs {
        name: "transfer_hacd_single_to",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &DiaSingleTrs| false,
        req_sign: |_: &DiaSingleTrs| vec![],
        as_transfer_like: self,
        description: |this: &DiaSingleTrs| format!("Transfer 1 HACD ({}) to {}", this.diamond.to_readable(), addr_or_ptr_readable(&this.to)),

    }
}

base::impl_action_facts! {
    DiaFromToTrs {
        name: "transfer_hacd_from_to",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &DiaFromToTrs| false,
        req_sign: |this: &DiaFromToTrs| vec![this.from.clone()],
        as_transfer_like: self,
        description: |this: &DiaFromToTrs| format!("Transfer {} HACD ({}) from {} to {}", this.diamonds.length(), this.diamonds.splitstr(), addr_or_ptr_readable(&this.from), addr_or_ptr_readable(&this.to)),

    }
}

base::impl_action_facts! {
    DiaToTrs {
        name: "transfer_hacd_to",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &DiaToTrs| false,
        req_sign: |_: &DiaToTrs| vec![],
        as_transfer_like: self,
        description: |this: &DiaToTrs| format!("Transfer {} HACD ({}) to {}", this.diamonds.length(), this.diamonds.splitstr(), addr_or_ptr_readable(&this.to)),

    }
}

base::impl_action_facts! {
    DiaFromTrs {
        name: "transfer_hacd_from",
        scope: ActScope::CALL,
        min_tx_type: 2,
        extra9: |_: &DiaFromTrs| false,
        req_sign: |this: &DiaFromTrs| vec![this.from.clone()],
        as_transfer_like: self,
        description: |this: &DiaFromTrs| format!("Transfer {} HACD ({}) from {}", this.diamonds.length(), this.diamonds.splitstr(), addr_or_ptr_readable(&this.from)),

    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use field::{Decode, ToJSON};

    #[test]
    fn derived_codec_round_trips_wire_and_json_fields() {
        let action = SatFromToTrs::new(Address::default(), Address::default(), Satoshi::from(7));
        let mut wire = action.encode();
        let action_size = wire.len();
        wire.extend_from_slice(&[0xaa, 0xbb]);
        let (decoded, used) = SatFromToTrs::decode(&wire).expect("decode action");
        assert_eq!(used, action_size);
        assert_eq!(decoded.encode(), wire[..action_size]);

        let json = action.to_json();
        assert_eq!(
            json,
            format!(
                "{{\"kind\":{},\"from\":{},\"to\":{},\"satoshi\":7}}",
                SatFromToTrs::KIND,
                Address::default().to_json(),
                Address::default().to_json(),
            )
        );

        let wrong_kind = SatToTrs::new(Address::default(), Satoshi::from(7)).encode();
        assert!(SatFromToTrs::decode(&wrong_kind).is_err());

        let decoded = <SatFromToTrs as base::ActionJsonCodec>::decode_json(&json)
            .expect("decode action json");
        assert_eq!(decoded.encode(), action.encode());
        assert!(
            <SatFromToTrs as base::ActionJsonCodec>::decode_json(
                "{\"kind\":12,\"from\":0,\"from\":0,\"to\":0,\"satoshi\":7}"
            )
            .is_err()
        );
        assert!(
            <SatFromToTrs as base::ActionJsonCodec>::decode_json(
                "{\"kind\":12,\"from\":0,\"to\":0}"
            )
            .is_err()
        );
    }
}
