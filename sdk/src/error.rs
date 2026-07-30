use std::fmt::Display;

use wasm_bindgen::JsValue;

pub(crate) fn js_error(error: impl Display) -> JsValue {
    JsValue::from_str(&error.to_string())
}

pub(crate) fn fault(context: &str, error: impl Display) -> sys::Error {
    sys::Error::fault(format!("{}: {}", context, error))
}
