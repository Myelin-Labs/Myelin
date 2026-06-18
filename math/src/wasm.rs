pub use js_sys::BigInt;
pub use wasm_bindgen::{JsCast, JsValue};

pub fn js_value_to_vec_u8(value: &JsValue) -> Result<Vec<u8>, crate::Error> {
    if let Some(text) = value.as_string() {
        return hex_string_to_bytes(&text);
    }

    if value.is_object() {
        return Ok(js_sys::Uint8Array::new(value).to_vec());
    }

    Err(crate::Error::NotCompatible)
}

fn hex_string_to_bytes(text: &str) -> Result<Vec<u8>, crate::Error> {
    let hex = text.strip_prefix("0x").unwrap_or(text);
    if hex.is_empty() {
        return Ok(Vec::new());
    }

    let normalised = if hex.len() % 2 == 0 { hex.to_owned() } else { format!("0{hex}") };
    let mut out = vec![0u8; normalised.len() / 2];
    faster_hex::hex_decode(normalised.as_bytes(), &mut out)?;
    Ok(out)
}
