//! Message / Blob actions.

use field::{BytesW1, BytesW2};

const TX_BLOB_DESCRIPTION_PREVIEW_LEN: usize = 128;

fn tx_message_description(data: &BytesW1) -> String {
    sys::bytes_to_readable_string_or_hex(data.as_ref())
}

fn tx_blob_description(data: &BytesW2) -> String {
    let bytes = data.as_ref();
    let preview_len = bytes.len().min(TX_BLOB_DESCRIPTION_PREVIEW_LEN);
    format!(
        "{}... ({})",
        hex::encode(&bytes[..preview_len]),
        bytes.len()
    )
}

base::action_simple! { Message, 0x0401, 2, GUARD, {
    data: BytesW1
}, this, {
    blob,
    description: tx_message_description(&this.data)
}}

base::action_simple! { Blob, 0x0402, 2, GUARD, {
    data: BytesW2
}, this, {
    blob,
    description: tx_blob_description(&this.data)
}}

#[cfg(test)]
mod tests {
    use super::*;
    use base::Action;

    #[test]
    fn tx_message_description_uses_readable_ascii_or_hex() {
        let ascii = Message::new(BytesW1::from(b"hello".to_vec()).unwrap());
        assert_eq!(ascii.description(), "hello");

        let non_ascii = Message::new(BytesW1::from(vec![0x68, 0xc3, 0xa9]).unwrap());
        assert_eq!(non_ascii.description(), "68c3a9");

        let non_printable = Message::new(BytesW1::from(vec![b'a', 0, b'b']).unwrap());
        assert_eq!(non_printable.description(), "610062");
    }

    #[test]
    fn tx_blob_description_includes_length_and_truncates_preview() {
        let short = Blob::new(BytesW2::from(vec![0x01, 0xab]).unwrap());
        assert_eq!(short.description(), "01ab... (2)");

        let exact =
            Blob::new(BytesW2::from(vec![0x5a; TX_BLOB_DESCRIPTION_PREVIEW_LEN]).unwrap());
        assert_eq!(
            exact.description(),
            format!("{}... (128)", "5a".repeat(128))
        );

        let long = Blob::new(BytesW2::from(vec![0x5a; 129]).unwrap());
        assert_eq!(long.description(), format!("{}... (129)", "5a".repeat(128)));
    }
}
