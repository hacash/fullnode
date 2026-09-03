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

base::action_simple! { TransferHacTo, 1, 1, CALL, {
    to: AddrOrPtr,
    hacash: Amount
}, this, {
    transfer: (to = to, payload = Hac(hacash)),
    description: format!("Transfer {} HAC to {}", this.hacash.to_unit_string("HAC"), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { TransferHacFrom, 13, 1, CALL, {
    from: AddrOrPtr,
    hacash: Amount
}, this, {
    req_sign: vec![this.from.clone()],
    transfer: (from = from, payload = Hac(hacash)),
    description: format!("Transfer {} HAC from {}", this.hacash.to_unit_string("HAC"), addr_or_ptr_readable(&this.from))
}}

base::action_simple! { TransferHacFromTo, 14, 1, CALL, {
    from: AddrOrPtr,
    to: AddrOrPtr,
    hacash: Amount
}, this, {
    req_sign: vec![this.from.clone()],
    transfer: (to = to, from = from, payload = Hac(hacash)),
    description: format!("Transfer {} HAC from {} to {}", this.hacash.to_unit_string("HAC"), addr_or_ptr_readable(&this.from), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { TransferSatTo, 10, 2, CALL, {
    to: AddrOrPtr,
    satoshi: Satoshi
}, this, {
    transfer: (to = to, payload = Sat(satoshi)),
    description: format!("Transfer {} SAT to {}", this.satoshi.uint(), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { TransferSatFrom, 11, 2, CALL, {
    from: AddrOrPtr,
    satoshi: Satoshi
}, this, {
    req_sign: vec![this.from.clone()],
    transfer: (from = from, payload = Sat(satoshi)),
    description: format!("Transfer {} SAT from {}", this.satoshi.uint(), addr_or_ptr_readable(&this.from))
}}

base::action_simple! { TransferSatFromTo, 12, 2, CALL, {
    from: AddrOrPtr,
    to: AddrOrPtr,
    satoshi: Satoshi
}, this, {
    req_sign: vec![this.from.clone()],
    transfer: (to = to, from = from, payload = Sat(satoshi)),
    description: format!("Transfer {} SAT from {} to {}", this.satoshi.uint(), addr_or_ptr_readable(&this.from), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { TransferAssetTo, 17, 2, CALL, {
    to: AddrOrPtr,
    asset: AssetAmt
}, this, {
    extra9: true,
    transfer: (to = to, payload = Asset(asset)),
    description: format!("Transfer {{{}:{}}} to {}", this.asset.serial.uint(), this.asset.amount.uint(), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { TransferAssetFrom, 18, 2, CALL, {
    from: AddrOrPtr,
    asset: AssetAmt
}, this, {
    extra9: true,
    req_sign: vec![this.from.clone()],
    transfer: (from = from, payload = Asset(asset)),
    description: format!("Transfer {{{}:{}}} from {}", this.asset.serial.uint(), this.asset.amount.uint(), addr_or_ptr_readable(&this.from))
}}

base::action_simple! { TransferAssetFromTo, 19, 2, CALL, {
    from: AddrOrPtr,
    to: AddrOrPtr,
    asset: AssetAmt
}, this, {
    extra9: true,
    req_sign: vec![this.from.clone()],
    transfer: (to = to, from = from, payload = Asset(asset)),
    description: format!("Transfer {{{}:{}}} from {} to {}", this.asset.serial.uint(), this.asset.amount.uint(), addr_or_ptr_readable(&this.from), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { TransferHacdSingleTo, 5, 2, CALL, {
    diamond: DiamondName,
    to: AddrOrPtr
}, this, {
    validate: "Self::validate_codec",
    transfer: (to = to, payload = Hacd(1, diamond)),
    description: format!("Transfer 1 HACD ({}) to {}", this.diamond.to_readable(), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { TransferHacdFromTo, 6, 2, CALL, {
    from: AddrOrPtr,
    to: AddrOrPtr,
    diamonds: DiamondNameListMax200
}, this, {
    validate: "Self::validate_codec",
    req_sign: vec![this.from.clone()],
    transfer: (to = to, from = from, payload = Hacd(diamonds.length(), diamonds)),
    description: format!("Transfer {} HACD ({}) from {} to {}", this.diamonds.length(), this.diamonds.splitstr(), addr_or_ptr_readable(&this.from), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { TransferHacdTo, 7, 2, CALL, {
    to: AddrOrPtr,
    diamonds: DiamondNameListMax200
}, this, {
    validate: "Self::validate_codec",
    transfer: (to = to, payload = Hacd(diamonds.length(), diamonds)),
    description: format!("Transfer {} HACD ({}) to {}", this.diamonds.length(), this.diamonds.splitstr(), addr_or_ptr_readable(&this.to))
}}

base::action_simple! { TransferHacdFrom, 8, 2, CALL, {
    from: AddrOrPtr,
    diamonds: DiamondNameListMax200
}, this, {
    validate: "Self::validate_codec",
    req_sign: vec![this.from.clone()],
    transfer: (from = from, payload = Hacd(diamonds.length(), diamonds)),
    description: format!("Transfer {} HACD ({}) from {}", this.diamonds.length(), this.diamonds.splitstr(), addr_or_ptr_readable(&this.from))
}}

impl TransferHacdSingleTo {
    fn validate_codec(&self) -> Ret<()> {
        DiamondName::check_bytes(self.diamond.as_ref())
    }
}

impl TransferHacdFromTo {
    fn validate_codec(&self) -> Ret<()> {
        self.diamonds.check().map(|_| ())
    }
}

impl TransferHacdTo {
    fn validate_codec(&self) -> Ret<()> {
        self.diamonds.check().map(|_| ())
    }
}

impl TransferHacdFrom {
    fn validate_codec(&self) -> Ret<()> {
        self.diamonds.check().map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use field::{Address, Decode, Encode, FromJSON, ToJSON};

    #[test]
    fn derived_codec_round_trips_wire_and_json_fields() {
        let action =
            TransferSatFromTo::new(Address::default(), Address::default(), Satoshi::from(7));
        let mut wire = action.encode();
        let action_size = wire.len();
        wire.extend_from_slice(&[0xaa, 0xbb]);
        let (decoded, used) = TransferSatFromTo::decode(&wire).expect("decode action");
        assert_eq!(used, action_size);
        assert_eq!(action.size(), action_size);
        assert_eq!(decoded.encode(), wire[..action_size]);

        let json = action.to_json();
        assert_eq!(
            json,
            format!(
                "{{\"kind\":{},\"from\":{},\"to\":{},\"satoshi\":7}}",
                TransferSatFromTo::KIND,
                Address::default().to_json(),
                Address::default().to_json(),
            )
        );

        let wrong_kind = TransferSatTo::new(Address::default(), Satoshi::from(7)).encode();
        assert!(TransferSatFromTo::decode(&wrong_kind).is_err());

        let mut decoded = TransferSatFromTo::default();
        decoded.from_json(&json).expect("decode action json");
        assert_eq!(decoded.encode(), action.encode());
        assert!(
            TransferSatFromTo::default()
                .from_json("{\"kind\":12,\"from\":0,\"from\":0,\"to\":0,\"satoshi\":7}")
                .is_err()
        );
        assert!(
            TransferSatFromTo::default()
                .from_json("{\"kind\":12,\"from\":0,\"to\":0}")
                .is_err()
        );
    }

    fn addr(n: u8) -> Address {
        let mut bytes = [0u8; 21];
        bytes[1] = n;
        Address::from(bytes)
    }

    #[test]
    fn transfer_intent_shapes_match_the_thirteen_kinds() {
        use base::{Action, TransferAsset, TransferIntent};

        fn shape(intent: TransferIntent) -> (bool, bool, &'static str) {
            let tag = match &intent.asset {
                TransferAsset::Hac(_) => "hac",
                TransferAsset::Sat(_) => "sat",
                TransferAsset::Diamond(_) => "diamond",
                TransferAsset::Asset(_) => "asset",
            };
            (intent.from.is_some(), intent.to.is_some(), tag)
        }

        let a = addr(1);
        let b = addr(2);
        let hac = Amount::from("1:244").unwrap();
        let sat = Satoshi::from(7);
        let diamonds = DiamondNameListMax200::from_readable("WTYUIA,HYXYHY").unwrap();
        let diamond = diamonds.as_list()[0];
        let asset = AssetAmt {
            serial: field::Fold64::from(1).expect("serial"),
            amount: field::Fold64::from(9).expect("amount"),
        };

        let shapes = vec![
            (
                TransferHacTo::KIND,
                shape(
                    TransferHacTo::new(a, hac.clone())
                        .transfer_intent()
                        .unwrap(),
                ),
            ),
            (
                TransferHacFrom::KIND,
                shape(
                    TransferHacFrom::new(a, hac.clone())
                        .transfer_intent()
                        .unwrap(),
                ),
            ),
            (
                TransferHacFromTo::KIND,
                shape(TransferHacFromTo::new(a, b, hac).transfer_intent().unwrap()),
            ),
            (
                TransferSatTo::KIND,
                shape(TransferSatTo::new(a, sat).transfer_intent().unwrap()),
            ),
            (
                TransferSatFrom::KIND,
                shape(TransferSatFrom::new(a, sat).transfer_intent().unwrap()),
            ),
            (
                TransferSatFromTo::KIND,
                shape(TransferSatFromTo::new(a, b, sat).transfer_intent().unwrap()),
            ),
            (
                TransferAssetTo::KIND,
                shape(
                    TransferAssetTo::new(a, asset.clone())
                        .transfer_intent()
                        .unwrap(),
                ),
            ),
            (
                TransferAssetFrom::KIND,
                shape(
                    TransferAssetFrom::new(a, asset.clone())
                        .transfer_intent()
                        .unwrap(),
                ),
            ),
            (
                TransferAssetFromTo::KIND,
                shape(
                    TransferAssetFromTo::new(a, b, asset)
                        .transfer_intent()
                        .unwrap(),
                ),
            ),
            (
                TransferHacdSingleTo::KIND,
                shape(
                    TransferHacdSingleTo::new(diamond, a)
                        .transfer_intent()
                        .unwrap(),
                ),
            ),
            (
                TransferHacdFromTo::KIND,
                shape(
                    TransferHacdFromTo::new(a, b, diamonds.clone())
                        .transfer_intent()
                        .unwrap(),
                ),
            ),
            (
                TransferHacdTo::KIND,
                shape(
                    TransferHacdTo::new(a, diamonds.clone())
                        .transfer_intent()
                        .unwrap(),
                ),
            ),
            (
                TransferHacdFrom::KIND,
                shape(
                    TransferHacdFrom::new(a, diamonds)
                        .transfer_intent()
                        .unwrap(),
                ),
            ),
        ];
        assert_eq!(
            shapes,
            vec![
                (1, (false, true, "hac")),
                (13, (true, false, "hac")),
                (14, (true, true, "hac")),
                (10, (false, true, "sat")),
                (11, (true, false, "sat")),
                (12, (true, true, "sat")),
                (17, (false, true, "asset")),
                (18, (true, false, "asset")),
                (19, (true, true, "asset")),
                (5, (false, true, "diamond")),
                (6, (true, true, "diamond")),
                (7, (false, true, "diamond")),
                (8, (true, false, "diamond")),
            ]
        );
    }
}
