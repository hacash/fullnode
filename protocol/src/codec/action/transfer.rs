//! Hac / Sat / Asset / Diamond transfer actions.

use base::AddrOrPtr;
use field::{Amount, AssetAmt, DiamondName, DiamondNameListMax200, Satoshi};
use sys::Ret;

/// Readable rendering of a wire destination / source (address or pointer).
pub(super) fn addr_or_ptr_readable(ptr: &AddrOrPtr) -> String {
    match ptr {
        AddrOrPtr::Addr(addr) => addr.to_readable(),
        AddrOrPtr::Ptr(index) => format!("<address pointer {}>", index),
    }
}

base::action_simple! { HacToTrs, 1, 1, CALL, {
    to: AddrOrPtr,
    hacash: Amount
}, this, {
    name: "transfer_hac_to",
    transfer: (to = to, payload = Hac(hacash)),
    description: format!("Transfer {} HAC to {}", this.hacash.to_unit_string("HAC"), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { HacFromTrs, 13, 1, CALL, {
    from: AddrOrPtr,
    hacash: Amount
}, this, {
    name: "transfer_hac_from",
    req_sign: vec![this.from.clone()],
    transfer: (from = from, payload = Hac(hacash)),
    description: format!("Transfer {} HAC from {}", this.hacash.to_unit_string("HAC"), addr_or_ptr_readable(&this.from))
}}

base::action_simple! { HacFromToTrs, 14, 1, CALL, {
    from: AddrOrPtr,
    to: AddrOrPtr,
    hacash: Amount
}, this, {
    name: "transfer_hac_from_to",
    req_sign: vec![this.from.clone()],
    transfer: (to = to, from = from, payload = Hac(hacash)),
    description: format!("Transfer {} HAC from {} to {}", this.hacash.to_unit_string("HAC"), addr_or_ptr_readable(&this.from), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { SatToTrs, 10, 2, CALL, {
    to: AddrOrPtr,
    satoshi: Satoshi
}, this, {
    name: "transfer_sat_to",
    transfer: (to = to, payload = Sat(satoshi)),
    description: format!("Transfer {} SAT to {}", this.satoshi.uint(), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { SatFromTrs, 11, 2, CALL, {
    from: AddrOrPtr,
    satoshi: Satoshi
}, this, {
    name: "transfer_sat_from",
    req_sign: vec![this.from.clone()],
    transfer: (from = from, payload = Sat(satoshi)),
    description: format!("Transfer {} SAT from {}", this.satoshi.uint(), addr_or_ptr_readable(&this.from))
}}

base::action_simple! { SatFromToTrs, 12, 2, CALL, {
    from: AddrOrPtr,
    to: AddrOrPtr,
    satoshi: Satoshi
}, this, {
    name: "transfer_sat_from_to",
    req_sign: vec![this.from.clone()],
    transfer: (to = to, from = from, payload = Sat(satoshi)),
    description: format!("Transfer {} SAT from {} to {}", this.satoshi.uint(), addr_or_ptr_readable(&this.from), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { AssetToTrs, 17, 2, CALL, {
    to: AddrOrPtr,
    asset: AssetAmt
}, this, {
    name: "transfer_asset_to",
    extra9: true,
    transfer: (to = to, payload = Asset(asset.serial, asset.amount)),
    description: format!("Transfer {{{}:{}}} to {}", this.asset.serial.uint(), this.asset.amount.uint(), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { AssetFromTrs, 18, 2, CALL, {
    from: AddrOrPtr,
    asset: AssetAmt
}, this, {
    name: "transfer_asset_from",
    extra9: true,
    req_sign: vec![this.from.clone()],
    transfer: (from = from, payload = Asset(asset.serial, asset.amount)),
    description: format!("Transfer {{{}:{}}} from {}", this.asset.serial.uint(), this.asset.amount.uint(), addr_or_ptr_readable(&this.from))
}}

base::action_simple! { AssetFromToTrs, 19, 2, CALL, {
    from: AddrOrPtr,
    to: AddrOrPtr,
    asset: AssetAmt
}, this, {
    name: "transfer_asset_from_to",
    extra9: true,
    req_sign: vec![this.from.clone()],
    transfer: (to = to, from = from, payload = Asset(asset.serial, asset.amount)),
    description: format!("Transfer {{{}:{}}} from {} to {}", this.asset.serial.uint(), this.asset.amount.uint(), addr_or_ptr_readable(&this.from), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { DiaSingleTrs, 5, 2, CALL, {
    diamond: DiamondName,
    to: AddrOrPtr
}, this, {
    name: "transfer_hacd_single_to",
    validate: "Self::validate_codec",
    transfer: (to = to, payload = Hacd(1, diamond)),
    description: format!("Transfer 1 HACD ({}) to {}", this.diamond.to_readable(), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { DiaFromToTrs, 6, 2, CALL, {
    from: AddrOrPtr,
    to: AddrOrPtr,
    diamonds: DiamondNameListMax200
}, this, {
    name: "transfer_hacd_from_to",
    validate: "Self::validate_codec",
    req_sign: vec![this.from.clone()],
    transfer: (to = to, from = from, payload = Hacd(diamonds.length(), diamonds)),
    description: format!("Transfer {} HACD ({}) from {} to {}", this.diamonds.length(), this.diamonds.splitstr(), addr_or_ptr_readable(&this.from), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { DiaToTrs, 7, 2, CALL, {
    to: AddrOrPtr,
    diamonds: DiamondNameListMax200
}, this, {
    name: "transfer_hacd_to",
    validate: "Self::validate_codec",
    transfer: (to = to, payload = Hacd(diamonds.length(), diamonds)),
    description: format!("Transfer {} HACD ({}) to {}", this.diamonds.length(), this.diamonds.splitstr(), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { DiaFromTrs, 8, 2, CALL, {
    from: AddrOrPtr,
    diamonds: DiamondNameListMax200
}, this, {
    name: "transfer_hacd_from",
    validate: "Self::validate_codec",
    req_sign: vec![this.from.clone()],
    transfer: (from = from, payload = Hacd(diamonds.length(), diamonds)),
    description: format!("Transfer {} HACD ({}) from {}", this.diamonds.length(), this.diamonds.splitstr(), addr_or_ptr_readable(&this.from))
}}

impl DiaSingleTrs {
    fn validate_codec(&self) -> Ret<()> {
        DiamondName::check_bytes(self.diamond.as_ref())
    }
}

impl DiaFromToTrs {
    fn validate_codec(&self) -> Ret<()> {
        self.diamonds.check().map(|_| ())
    }
}

impl DiaToTrs {
    fn validate_codec(&self) -> Ret<()> {
        self.diamonds.check().map(|_| ())
    }
}

impl DiaFromTrs {
    fn validate_codec(&self) -> Ret<()> {
        self.diamonds.check().map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use field::{Address, Decode, Encode, ToJSON};

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
