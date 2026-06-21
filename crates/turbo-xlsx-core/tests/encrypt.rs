#![cfg(feature = "encrypt")]
//! Agile-encryption tests. The structural checks run in-process; full
//! decryptability is verified out-of-band by `msoffcrypto-tool` (see
//! `benches`/CI), driven by the `TURBO_ENC_OUT` dump below.

use turbo_xlsx_core::{write_from_json_str, WriteOptions};

const WB: &str = r#"{"sheets":[{"name":"S","rows":[
    {"cells":[{"type":"string","value":"hello"},{"type":"number","value":42}]}
]}]}"#;

fn encrypted(password: &str) -> Vec<u8> {
    let opts = WriteOptions {
        password: Some(password.into()),
        ..Default::default()
    };
    write_from_json_str(WB, &opts).unwrap().xlsx
}

#[test]
fn password_produces_a_cfb_container() {
    let bytes = encrypted("s3cret");
    // CFB/OLE2 magic — an encrypted .xlsx is a compound file, not a zip.
    assert_eq!(
        &bytes[..8],
        &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]
    );
    // Dump for the external msoffcrypto-tool round-trip check.
    if let Ok(path) = std::env::var("TURBO_ENC_OUT") {
        std::fs::write(path, &bytes).unwrap();
    }
}

#[test]
fn no_password_stays_a_plain_zip() {
    let plain = write_from_json_str(WB, &WriteOptions::default())
        .unwrap()
        .xlsx;
    assert_eq!(&plain[..2], b"PK"); // unencrypted OPC zip
}

#[test]
fn empty_password_is_not_encrypted() {
    let opts = WriteOptions {
        password: Some(String::new()),
        ..Default::default()
    };
    let bytes = write_from_json_str(WB, &opts).unwrap().xlsx;
    assert_eq!(&bytes[..2], b"PK");
}

#[test]
fn encryption_is_randomized() {
    // Random salts/keys ⇒ two encryptions of the same input differ.
    assert_ne!(encrypted("pw"), encrypted("pw"));
}
