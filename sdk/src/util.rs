


#[wasm_bindgen]
pub fn hac_to_unit(stuff: &str, unit: u8) -> Ret<f64> {
    Amount::from(stuff).map(|a|unsafe{a.to_unit_float(unit)})
}

#[wasm_bindgen]
pub fn hac_to_mei(stuff: &str) -> Ret<f64> {
    hac_to_unit(stuff, UNIT_MEI)
}

