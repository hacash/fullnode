use sys::{Ret, errf};

use crate::codec::{Decode, Encode};
use crate::json::{
    FromJSON, JSONBinaryFormat, JSONFormater, ToJSON, json_decode_binary,
    json_expect_quoted_decoded, json_expect_unquoted, json_split_array, json_split_object,
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
                    *self = <$name>::from(v);
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
        match fmt.binary {
            JSONBinaryFormat::Hex => format!("\"0x{}\"", hex::encode(self.0)),
            JSONBinaryFormat::Base64 => {
                use base64::prelude::*;
                format!("\"b64:{}\"", BASE64_STANDARD.encode(self.0))
            }
        }
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
        format!("\"{}\"", self.to_readable())
    }
}

impl FromJSON for Address {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        let raw = json_expect_quoted_decoded(json)?;
        if let Ok(address) = Address::from_readable(raw.trim()) {
            *self = address;
            return Ok(());
        }
        let data = json_decode_binary(json)?;
        if data.len() != Address::SIZE {
            return errf!(
                "Address size invalid: expected {}, got {}",
                Address::SIZE,
                data.len()
            );
        }
        let address = Address::from(data.try_into().expect("Address size checked"));
        if !address.is_supported() {
            return errf!("address version {} not supported", address.version());
        }
        *self = address;
        Ok(())
    }
}

impl ToJSON for Bool {
    fn to_json_fmt(&self, _fmt: &JSONFormater) -> String {
        if self.is_true() { "1" } else { "0" }.to_owned()
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
                    match fmt.binary {
                        JSONBinaryFormat::Hex => format!("\"0x{}\"", hex::encode(self.as_ref())),
                        JSONBinaryFormat::Base64 => {
                            use base64::prelude::*;
                            format!("\"b64:{}\"", BASE64_STANDARD.encode(self.as_ref()))
                        }
                    }
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
        let value = if fmt.unit.is_empty() {
            self.to_fin_string()
        } else {
            self.to_unit_string(&fmt.unit)
        };
        format!("\"{}\"", value)
    }
}

impl FromJSON for Amount {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        *self = Amount::from(&json_expect_quoted_decoded(json)?)?;
        Ok(())
    }
}

impl ToJSON for Sign {
    fn to_json_fmt(&self, _fmt: &JSONFormater) -> String {
        format!("\"0x{}\"", hex::encode(self.encode()))
    }
}

impl FromJSON for Sign {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        let data = json_decode_binary(json)?;
        let (v, used) = Sign::decode(&data)?;
        if used != data.len() {
            return errf!("Sign JSON has {} trailing bytes", data.len() - used);
        }
        *self = v;
        Ok(())
    }
}

macro_rules! impl_list_json {
    ($($name:ident),+ $(,)?) => {
        $(
            impl<T: ToJSON> ToJSON for $name<T> {
                fn to_json_fmt(&self, fmt: &JSONFormater) -> String {
                    format!(
                        "[{}]",
                        self.0
                            .iter()
                            .map(|v| v.to_json_fmt(fmt))
                            .collect::<Vec<_>>()
                            .join(",")
                    )
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

impl ToJSON for DiamondName {
    fn to_json_fmt(&self, _fmt: &JSONFormater) -> String {
        format!("\"{}\"", self.to_readable())
    }
}

impl FromJSON for DiamondName {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        let raw = json_expect_quoted_decoded(json)?;
        if let Ok(name) = DiamondName::from_readable(raw.trim()) {
            *self = name;
            return Ok(());
        }
        let data = json_decode_binary(json)?;
        let name = DiamondName::from_readable(data)?;
        *self = name;
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

impl ToJSON for AssetAmt {
    fn to_json_fmt(&self, fmt: &JSONFormater) -> String {
        format!(
            "{{\"serial\":{},\"amount\":{}}}",
            self.serial.to_json_fmt(fmt),
            self.amount.to_json_fmt(fmt)
        )
    }
}

impl FromJSON for AssetAmt {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        let mut serial = self.serial;
        let mut amount = self.amount;
        let mut seen = std::collections::HashSet::new();
        for (key, value) in json_split_object(json)? {
            if !seen.insert(key) {
                return errf!("AssetAmt JSON field {} is duplicated", key);
            }
            match key {
                "serial" => serial.from_json(value)?,
                "amount" => amount.from_json(value)?,
                _ => {}
            }
        }
        *self = AssetAmt { serial, amount }.checked()?;
        Ok(())
    }
}

impl ToJSON for Balance {
    fn to_json_fmt(&self, fmt: &JSONFormater) -> String {
        format!(
            "{{\"hacash\":{},\"satoshi\":{},\"diamond\":{},\"assets\":{}}}",
            self.hacash.to_json_fmt(fmt),
            self.satoshi.to_json_fmt(fmt),
            self.diamond.to_json_fmt(fmt),
            self.assets.to_json_fmt(fmt)
        )
    }
}

impl FromJSON for Balance {
    fn from_json(&mut self, json: &str) -> Ret<()> {
        let mut hacash = self.hacash.clone();
        let mut satoshi = self.satoshi;
        let mut diamond = self.diamond;
        let mut assets = self.assets.clone();
        let mut seen = std::collections::HashSet::new();
        for (key, value) in json_split_object(json)? {
            if !seen.insert(key) {
                return errf!("Balance JSON field {} is duplicated", key);
            }
            match key {
                "hacash" => hacash.from_json(value)?,
                "satoshi" => satoshi.from_json(value)?,
                "diamond" => diamond.from_json(value)?,
                "assets" => assets.from_json(value)?,
                _ => {}
            }
        }
        Balance::check_assets(&assets)?;
        *self = Balance {
            hacash,
            satoshi,
            diamond,
            assets,
        };
        Ok(())
    }
}
