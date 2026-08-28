use sys::{Ret, errf};

use crate::json::{
    FromJSON, JSONBinaryFormat, JSONFormater, ToJSON, json_decode_binary,
    json_expect_quoted_decoded, json_expect_unquoted, json_split_array,
};
use crate::types::*;

macro_rules! impl_uint_json {
    ($($name:ty),+ $(,)?) => {
        $(
            impl ToJSON for $name {
                fn to_json_fmt(&self, _fmt: &JSONFormater) -> String {
                    self.uint().to_string()
                }
            }

            impl FromJSON for $name {
                fn from_json(&mut self, json: &str) -> Ret<()> {
                    let v = json_expect_unquoted(json)?
                        .parse()
                        .map_err(|_| sys::Error::normal(format!("cannot parse {}", stringify!($name))))?;
                    *self = <$name>::from_checked(v).ok_or_else(|| {
                        sys::Error::normal(format!(
                            "{} value {} exceeds max {}",
                            stringify!($name),
                            v,
                            <$name>::MAX
                        ))
                    })?;
                    Ok(())
                }
            }
        )+
    };
}

impl_uint_json!(
    Uint1, Uint2, Uint3, Uint4, Uint5, Uint6, Uint7, Uint8, Uint10, Uint12, Uint16
);

impl ToJSON for Timestamp {
    fn to_json_fmt(&self, _fmt: &JSONFormater) -> String {
        self.value().to_string()
    }
}

impl FromJSON for Timestamp {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        let value = json_expect_unquoted(json)?
            .parse()
            .map_err(|_| sys::Error::normal("cannot parse Timestamp"))?;
        *self = Timestamp::from_checked(value)?;
        Ok(())
    }
}

impl<const N: usize> ToJSON for Fixed<N> {
    fn to_json_fmt(&self, fmt: &JSONFormater) -> String {
        if N == 1 {
            return self.0[0].to_string();
        }
        let mut s = String::with_capacity(2 + N * 2);
        s.push('"');
        match fmt.binary {
            JSONBinaryFormat::Hex => {
                s.push_str("0x");
                s.push_str(&hex::encode(self.0));
            }
            JSONBinaryFormat::Base64 => {
                use base64::prelude::*;
                s.push_str("b64:");
                s.push_str(&BASE64_STANDARD.encode(self.0));
            }
        }
        s.push('"');
        s
    }
}

impl<const N: usize> FromJSON for Fixed<N> {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        if N == 1 && !json.trim().starts_with('"') {
            self.0[0] = json_expect_unquoted(json)?
                .parse()
                .map_err(|_| sys::Error::normal("cannot parse Fixed1"))?;
            return Ok(());
        }
        let data = json_decode_binary(json)?;
        if data.len() != N {
            return errf!(
                "Fixed<{}> size invalid: expected {} bytes, got {}",
                N,
                N,
                data.len()
            );
        }
        self.0.copy_from_slice(&data);
        Ok(())
    }
}

impl ToJSON for Address {
    fn to_json_fmt(&self, _fmt: &JSONFormater) -> String {
        let mut s = String::with_capacity(40);
        s.push('"');
        s.push_str(&self.to_readable());
        s.push('"');
        s
    }
}

impl FromJSON for Address {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        let raw = json_expect_quoted_decoded(json)?;
        let raw = raw.trim();
        if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
            let data = hex::decode(hex)
                .map_err(|e| sys::Error::normal(format!("address hex invalid: {e}")))?;
            if data.len() != Address::SIZE {
                return errf!(
                    "address hex length {} invalid, expected {}",
                    data.len(),
                    Address::SIZE
                );
            }
            let mut bytes = [0u8; Address::SIZE];
            bytes.copy_from_slice(&data);
            *self = Address::from(bytes);
            if !self.is_supported() {
                return errf!("address version {} not supported", self.version());
            }
            return Ok(());
        }
        *self = Address::from_readable(raw)?;
        Ok(())
    }
}

impl ToJSON for Bool {
    fn to_json_fmt(&self, _fmt: &JSONFormater) -> String {
        if self.is_true() { "true" } else { "false" }.to_owned()
    }
}

impl FromJSON for Bool {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        *self = match json_expect_unquoted(json)?.trim() {
            "1" | "true" | "True" | "TRUE" => Bool::new(true),
            "0" | "false" | "False" | "FALSE" => Bool::new(false),
            other => return errf!("cannot parse Bool from {}", other),
        };
        Ok(())
    }
}

macro_rules! impl_bytes_json {
    ($($name:ty),+ $(,)?) => {
        $(
            impl ToJSON for $name {
                fn to_json_fmt(&self, fmt: &JSONFormater) -> String {
                    let mut s = String::with_capacity(2 + self.as_ref().len() * 2);
                    s.push('"');
                    match fmt.binary {
                        JSONBinaryFormat::Hex => {
                            s.push_str("0x");
                            s.push_str(&hex::encode(self.as_ref()));
                        }
                        JSONBinaryFormat::Base64 => {
                            use base64::prelude::*;
                            s.push_str("b64:");
                            s.push_str(&BASE64_STANDARD.encode(self.as_ref()));
                        }
                    }
                    s.push('"');
                    s
                }
            }

            impl FromJSON for $name {
                fn from_json(&mut self, json: &str) -> Ret<()> {
                    *self = <$name>::from(json_decode_binary(json)?)?;
                    Ok(())
                }
            }
        )+
    };
}

impl_bytes_json!(BytesW1, BytesW2, BytesW4);

impl ToJSON for Fold64 {
    fn to_json_fmt(&self, _fmt: &JSONFormater) -> String {
        self.uint().to_string()
    }
}

impl FromJSON for Fold64 {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        *self = Fold64::from(
            json_expect_unquoted(json)?
                .parse()
                .map_err(|_| sys::Error::normal("cannot parse Fold64"))?,
        )?;
        Ok(())
    }
}

impl ToJSON for Amount {
    fn to_json_fmt(&self, fmt: &JSONFormater) -> String {
        let mut s = String::new();
        s.push('"');
        if fmt.unit.is_empty() {
            s.push_str(&self.to_fin_string());
        } else {
            s.push_str(&self.to_unit_string(&fmt.unit));
        }
        s.push('"');
        s
    }
}

impl FromJSON for Amount {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        *self = Amount::from(&json_expect_quoted_decoded(json)?)?;
        Ok(())
    }
}

macro_rules! impl_list_json {
    ($($name:ident),+ $(,)?) => {
        $(
            impl<T: ToJSON> ToJSON for $name<T> {
                fn to_json_fmt(&self, fmt: &JSONFormater) -> String {
                    let mut s = String::new();
                    s.push('[');
                    for (i, v) in self.0.iter().enumerate() {
                        if i > 0 {
                            s.push(',');
                        }
                        s.push_str(&v.to_json_fmt(fmt));
                    }
                    s.push(']');
                    s
                }
            }
        )+
    };
}

impl_list_json!(ListW1, ListW2);

macro_rules! impl_list_from_json {
    ($($name:ident),+ $(,)?) => {
        $(
            impl<T: Default + FromJSON> FromJSON for $name<T> {
                fn from_json(&mut self, json: &str) -> Ret<()> {
                    let items = json_split_array(json)?;
                    let mut values = Vec::with_capacity(items.len());
                    for item in items {
                        let mut value = T::default();
                        value.from_json(item)?;
                        values.push(value);
                    }
                    *self = Self::from(values)?;
                    Ok(())
                }
            }
        )+
    };
}

impl_list_from_json!(ListW1, ListW2);

/// Diamond name lists keep the legacy CSV-string JSON contract instead of the
/// generic list array: `"WTYUIA,HYXYHY"`. `FromJSON` accepts the quoted CSV
/// string (comma-separated or directly concatenated) and the array form.
macro_rules! impl_diamond_name_list_json {
    ($($name:ident),+ $(,)?) => {
        $(
            impl ToJSON for $name {
                fn to_json_fmt(&self, fmt: &JSONFormater) -> String {
                    let joined = if fmt.diamond_list_csv {
                        self.splitstr()
                    } else {
                        self.readable()
                    };
                    let mut s = String::with_capacity(
                        2 + self.length() * (DiamondName::SIZE + 1),
                    );
                    s.push('"');
                    s.push_str(&joined);
                    s.push('"');
                    s
                }
            }

            impl FromJSON for $name {
                fn from_json(&mut self, json: &str) -> Ret<()> {
                    let tmp = if json.trim().starts_with('[') {
                        let items = json_split_array(json)?;
                        let mut names = Vec::with_capacity(items.len());
                        for item in items {
                            let mut name = DiamondName::default();
                            name.from_json(item)?;
                            names.push(name);
                        }
                        $name::from(names)?
                    } else {
                        $name::from_readable(&json_expect_quoted_decoded(json)?)?
                    };
                    tmp.check()?;
                    *self = tmp;
                    Ok(())
                }
            }
        )+
    };
}

impl_diamond_name_list_json!(DiamondNameListMax200, DiamondNameListMax60000);

impl ToJSON for DiamondName {
    fn to_json_fmt(&self, _fmt: &JSONFormater) -> String {
        let mut s = String::with_capacity(self.as_ref().len() + 2);
        s.push('"');
        s.push_str(&self.to_readable());
        s.push('"');
        s
    }
}

impl FromJSON for DiamondName {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        let raw = json_expect_quoted_decoded(json)?;
        *self = DiamondName::from_readable(raw.trim())?;
        Ok(())
    }
}

impl ToJSON for SatoshiAuto {
    fn to_json_fmt(&self, _fmt: &JSONFormater) -> String {
        self.uint().to_string()
    }
}

impl FromJSON for SatoshiAuto {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        let mut value = Fold64::default();
        value.from_json(json)?;
        *self = SatoshiAuto::from_satoshi(&Satoshi::from(value.uint()))?;
        Ok(())
    }
}

impl ToJSON for DiamondNumberAuto {
    fn to_json_fmt(&self, _fmt: &JSONFormater) -> String {
        self.uint().to_string()
    }
}

impl FromJSON for DiamondNumberAuto {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        let mut value = Fold64::default();
        value.from_json(json)?;
        if value.uint() > DiamondNumber::MAX as u64 {
            return errf!("diamond number {} exceeds max", value.uint());
        }
        *self = DiamondNumberAuto::from_diamond(&DiamondNumber::from(value.uint() as u32));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{Decode, Encode};
    use crate::json::{FromJSON, ToJSON};

    fn sample_sign() -> Sign {
        Sign {
            publickey: Fixed::from([0x02; Sign::PUBLICKEY_SIZE]),
            signature: Fixed::from([0xab; Sign::SIGNATURE_SIZE]),
        }
    }

    fn sample_sign_json() -> String {
        let pk = hex::encode([0x02u8; 33]);
        let sig = hex::encode([0xabu8; 64]);
        format!("{{\"publickey\":\"0x{pk}\",\"signature\":\"0x{sig}\"}}")
    }

    #[test]
    fn uint_json_rejects_overflow_without_panic() {
        let mut v = Uint3::default();
        assert!(v.from_json("16777216").is_err());
        assert!(v.from_json("16777215").is_ok());

        let mut h = Uint5::default();
        assert!(h.from_json("1099511627776").is_err());
        assert!(h.from_json("1099511627775").is_ok());

        let mut big = Uint10::default();
        assert!(big.from_json("1208925819614629174706176").is_err());
        assert!(big.from_json("1208925819614629174706175").is_ok());

        // Uint1/2/4 parse through their exact underlying types and stay fine
        let mut small = Uint1::default();
        assert!(small.from_json("255").is_ok());
        assert!(small.from_json("256").is_err());
    }

    #[test]
    fn sign_json_matches_old_object_form() {
        let sign = sample_sign();
        let json = sign.to_json();
        let pk = sign.publickey.to_json();
        let sig = sign.signature.to_json();
        assert_eq!(json, format!("{{\"publickey\":{pk},\"signature\":{sig}}}"));

        let mut back = Sign::default();
        back.from_json(&json).unwrap();
        assert_eq!(back, sign);
    }

    #[test]
    fn sign_json_rejects_concatenated_hex_and_incomplete_objects() {
        let mut sign = Sign::default();
        let hex = format!(
            "\"0x{}{}\"",
            hex::encode([0x02u8; 33]),
            hex::encode([0xabu8; 64])
        );
        assert!(sign.from_json(&hex).is_err());
        assert!(sign.from_json("{}").is_err());
        assert!(
            sign.from_json(&format!(
                "{{\"publickey\":\"0x{}\"}}",
                hex::encode([0x02u8; 33])
            ))
            .is_err()
        );
        assert!(
            sign.from_json(&format!(
                "{{\"signature\":\"0x{}\"}}",
                hex::encode([0xabu8; 64])
            ))
            .is_err()
        );
        assert!(
            sign.from_json(&format!(
                "{{\"publickey\":\"0x{}\",\"signature\":\"0x{}\",\"junk\":1}}",
                hex::encode([0x02u8; 33]),
                hex::encode([0xabu8; 64])
            ))
            .is_err()
        );

        sign.from_json(&sample_sign_json()).unwrap();
        assert_eq!(sign, sample_sign());
    }

    #[test]
    fn sign_json_b64_follows_fixed_formatter() {
        use base64::prelude::*;

        let sign = sample_sign();
        let fmt = JSONFormater {
            binary: JSONBinaryFormat::Base64,
            ..Default::default()
        };
        let json = sign.to_json_fmt(&fmt);
        let pk = format!("b64:{}", BASE64_STANDARD.encode([0x02u8; 33]));
        let sig = format!("b64:{}", BASE64_STANDARD.encode([0xabu8; 64]));
        assert_eq!(
            json,
            format!("{{\"publickey\":\"{pk}\",\"signature\":\"{sig}\"}}")
        );

        let mut back = Sign::default();
        back.from_json(&json).unwrap();
        assert_eq!(back, sign);
    }

    #[test]
    fn address_json_accepts_hex_and_readable_forms() {
        let addr = Address::from([0u8; Address::SIZE]);
        let readable = format!("\"{}\"", addr.to_readable());
        let hex = format!("\"0x{}\"", hex::encode(addr.as_bytes()));

        let mut back = Address::default();
        back.from_json(&readable).unwrap();
        assert_eq!(back, addr);
        back.from_json(&hex).unwrap();
        assert_eq!(back, addr);
        back.from_json(&hex.replace("0x", "0X")).unwrap();
        assert_eq!(back, addr);

        // Wrong length and invalid hex are rejected.
        assert!(
            back.from_json(&format!("\"0x{}\"", hex::encode([0u8; 20])))
                .is_err()
        );
        assert!(back.from_json("\"0xzz\"").is_err());
        // Unsupported version byte is rejected like the readable path.
        let mut raw = [0u8; Address::SIZE];
        raw[0] = 0x09;
        assert!(
            back.from_json(&format!("\"0x{}\"", hex::encode(raw)))
                .is_err()
        );
    }

    #[test]
    fn diamond_name_json_rejects_wire_hex() {
        let mut name = DiamondName::default();
        assert!(name.from_json("\"0x575459554941\"").is_err());
        name.from_json("\"WTYUIA\"").unwrap();
        assert_eq!(name.to_readable(), "WTYUIA");
    }

    #[test]
    fn diamond_name_list_json_matches_old_csv_string() {
        let a = DiamondName::from_readable("WTYUIA").unwrap();
        let b = DiamondName::from_readable("HYXYHY").unwrap();
        let list = DiamondNameListMax200::from(vec![a, b]).unwrap();
        assert_eq!(list.to_json(), "\"WTYUIA,HYXYHY\"");

        let mut parsed = DiamondNameListMax200::default();
        parsed.from_json("\"WTYUIA,HYXYHY\"").unwrap();
        assert_eq!(parsed, list);
        parsed.from_json("\"WTYUIAHYXYHY\"").unwrap();
        assert_eq!(parsed, list);
        parsed.from_json("[\"WTYUIA\",\"HYXYHY\"]").unwrap();
        assert_eq!(parsed, list);

        assert!(parsed.from_json("[]").is_err());
        assert!(parsed.from_json("[\"WTYUIA\",\"WTYUIA\"]").is_err());

        let wide = DiamondNameListMax60000::from(vec![a, b]).unwrap();
        assert_eq!(wide.to_json(), "\"WTYUIA,HYXYHY\"");
        let mut wide_parsed = DiamondNameListMax60000::default();
        wide_parsed.from_json("\"WTYUIA,HYXYHY\"").unwrap();
        assert_eq!(wide_parsed, wide);

        let addrs = AddressW1::from(vec![Address::default()]).unwrap();
        assert!(addrs.to_json().starts_with('['));
    }

    #[test]
    fn diamond_name_list_json_csv_option() {
        let a = DiamondName::from_readable("WTYUIA").unwrap();
        let b = DiamondName::from_readable("HYXYHY").unwrap();
        let list = DiamondNameListMax200::from(vec![a, b]).unwrap();

        assert_eq!(list.to_json(), "\"WTYUIA,HYXYHY\"");
        let no_csv = JSONFormater {
            diamond_list_csv: false,
            ..Default::default()
        };
        assert_eq!(list.to_json_fmt(&no_csv), "\"WTYUIAHYXYHY\"");
        // The concatenated output is still accepted on input.
        let mut back = DiamondNameListMax200::default();
        back.from_json("\"WTYUIAHYXYHY\"").unwrap();
        assert_eq!(back, list);
    }

    #[test]
    fn bool_json_emits_true_false_and_accepts_legacy_digits() {
        assert_eq!(Bool::new(true).to_json(), "true");
        assert_eq!(Bool::new(false).to_json(), "false");

        let mut value = Bool::new(false);
        for input in ["true", "True", "TRUE", "1"] {
            value.from_json(input).unwrap();
            assert!(value.is_true(), "{input}");
        }
        for input in ["false", "False", "FALSE", "0"] {
            value.from_json(input).unwrap();
            assert!(!value.is_true(), "{input}");
        }
        assert!(value.from_json("\"true\"").is_err());
        assert!(value.from_json("2").is_err());
        assert!(value.from_json("yes").is_err());

        // Binary contract is still a single 0/1 byte.
        assert_eq!(Bool::new(true).encode(), vec![1]);
        assert_eq!(Bool::new(false).encode(), vec![0]);
        assert!(Bool::decode(&[2]).is_err());
    }
}
