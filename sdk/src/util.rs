use field::{Amount, UNIT_MEI};
use wasm_bindgen::prelude::*;

use crate::error::js_error;

fn hac_to_unit_inner(stuff: &str, unit: u8) -> sys::Ret<f64> {
    if unit > UNIT_MEI {
        return sys::errf!("unit {} out of range, max {}", unit, UNIT_MEI);
    }
    Amount::from(stuff).map(|amount| amount.to_unit_float(unit))
}

#[wasm_bindgen]
pub fn hac_to_unit(stuff: &str, unit: u8) -> Result<f64, JsValue> {
    hac_to_unit_inner(stuff, unit).map_err(js_error)
}

#[wasm_bindgen]
pub fn hac_to_mei(stuff: &str) -> Result<f64, JsValue> {
    hac_to_unit(stuff, UNIT_MEI)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_hac_amounts_for_js_numbers() {
        assert_eq!(hac_to_unit_inner("12:244", UNIT_MEI).unwrap(), 0.0012);
        assert!(hac_to_unit_inner("1", UNIT_MEI + 1).is_err());
    }
}
