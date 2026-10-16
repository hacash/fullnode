//! Message / Blob actions.

use field::descfmt::{PAYLOAD_PREVIEW_MAX_BYTES, hex_preview, user_text};
use field::{BytesW1, BytesW2};

fn tx_message_description(data: &BytesW1) -> String {
    format!(
        "Attach message ({} bytes): {}",
        data.as_ref().len(),
        user_text(data.as_ref())
    )
}

fn tx_blob_description(data: &BytesW2) -> String {
    let bytes = data.as_ref();
    format!(
        "Attach blob ({} bytes): {}",
        bytes.len(),
        hex_preview(bytes, PAYLOAD_PREVIEW_MAX_BYTES)
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
    fn tx_message_description_labels_and_bounds_the_payload() {
        let ascii = Message::new(BytesW1::from(b"hello".to_vec()).unwrap());
        assert_eq!(ascii.description(), "Attach message (5 bytes): \"hello\"");

        // quotes and backslashes inside the payload are escaped, never raw
        let quoted = Message::new(BytesW1::from(b"say \"hi\"".to_vec()).unwrap());
        assert_eq!(
            quoted.description(),
            "Attach message (8 bytes): \"say \\\"hi\\\"\""
        );

        let non_ascii = Message::new(BytesW1::from(vec![0x68, 0xc3, 0xa9]).unwrap());
        assert_eq!(non_ascii.description(), "Attach message (3 bytes): 0x68c3a9");

        let non_printable = Message::new(BytesW1::from(vec![b'a', 0, b'b']).unwrap());
        assert_eq!(non_printable.description(), "Attach message (3 bytes): 0x610062");
    }

    #[test]
    fn tx_blob_description_includes_length_and_truncates_preview() {
        let short = Blob::new(BytesW2::from(vec![0x01, 0xab]).unwrap());
        assert_eq!(short.description(), "Attach blob (2 bytes): 0x01ab");

        let exact = Blob::new(BytesW2::from(vec![0x5a; PAYLOAD_PREVIEW_MAX_BYTES]).unwrap());
        assert_eq!(
            exact.description(),
            format!("Attach blob (128 bytes): 0x{}", "5a".repeat(128))
        );

        let long = Blob::new(BytesW2::from(vec![0x5a; 129]).unwrap());
        assert_eq!(
            long.description(),
            format!("Attach blob (129 bytes): 0x{}...", "5a".repeat(128))
        );
    }
}
