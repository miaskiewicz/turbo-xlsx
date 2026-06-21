//! XLSX password protection — **ECMA-376 Agile Encryption** (MS-OFFCRYPTO).
//!
//! Excel/LibreOffice "encrypt with password" does not zip-encrypt; it wraps the
//! whole OPC package in a **CFB/OLE2** compound file with two streams —
//! `EncryptionInfo` (an XML descriptor) and `EncryptedPackage` (the AES-encrypted
//! zip). This module produces exactly that, the modern *agile* variant:
//! AES-256-CBC for the data, SHA-512 password key derivation (100k spins), and an
//! HMAC-SHA-512 integrity tag.
//!
//! It is behind the off-by-default `encrypt` feature and uses the vetted
//! RustCrypto crates (the base build stays dependency-free). The module is
//! excluded from the 100% line-coverage gate and validated functionally by
//! round-tripping through `msoffcrypto-tool`.

use aes::Aes256;
use cbc::cipher::{block_padding::NoPadding, BlockEncryptMut, KeyIvInit};
use cbc::Encryptor;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha512};
use std::io::Write;

use crate::error::{ErrorCode, TurboXlsxError};

type Result<T> = std::result::Result<T, TurboXlsxError>;
type HmacSha512 = Hmac<Sha512>;

const SPIN_COUNT: u32 = 100_000;
const SEGMENT: usize = 4096;

// Well-known block keys (MS-OFFCRYPTO §2.3.4.x): each derives a distinct AES key
// or IV from the same password hash / package key.
const BK_VERIFIER_INPUT: [u8; 8] = [0xfe, 0xa7, 0xd2, 0x76, 0x3b, 0x4b, 0x9e, 0x79];
const BK_VERIFIER_VALUE: [u8; 8] = [0xd7, 0xaa, 0x0f, 0x6d, 0x30, 0x61, 0x34, 0x4e];
const BK_KEY_VALUE: [u8; 8] = [0x14, 0x6e, 0x0b, 0xe7, 0xab, 0xac, 0xd0, 0xd6];
const BK_HMAC_KEY: [u8; 8] = [0x5f, 0xb2, 0xad, 0x01, 0x0c, 0xb9, 0xe1, 0xf6];
const BK_HMAC_VALUE: [u8; 8] = [0xa0, 0x67, 0x7f, 0x02, 0xb2, 0x2c, 0x84, 0x33];

/// A fatal encryption error.
fn enc_err(message: impl Into<String>) -> TurboXlsxError {
    TurboXlsxError::new(ErrorCode::Encryption, message)
}

/// Encrypt an OPC package (`.xlsx` zip bytes) with `password`, returning the
/// CFB-wrapped, agile-encrypted container Excel will open with that password.
pub fn encrypt(package: &[u8], password: &str) -> Result<Vec<u8>> {
    let r = gen_random()?;
    let key_data_salt: [u8; 16] = slice(&r, 0, 16);
    let package_key: [u8; 32] = slice(&r, 16, 32);
    let pw_salt: [u8; 16] = slice(&r, 48, 16);
    let verifier: [u8; 16] = slice(&r, 64, 16);
    let hmac_key: [u8; 64] = slice(&r, 80, 64);

    let enc_package = encrypt_package(package, &package_key, &key_data_salt);
    let material = key_material(password, &pw_salt, &package_key, &verifier);
    let integrity = data_integrity(&package_key, &key_data_salt, &hmac_key, &enc_package);

    let info = encryption_info(&key_data_salt, &pw_salt, &material, &integrity);
    build_cfb(&info, &enc_package)
}

/// One CSPRNG draw covering every random field (salts + keys + verifier).
fn gen_random() -> Result<[u8; 144]> {
    let mut buf = [0u8; 144];
    getrandom::getrandom(&mut buf).map_err(|e| enc_err(format!("rng: {e}")))?;
    Ok(buf)
}

/// A fixed-size sub-array of `buf` starting at `off`.
fn slice<const N: usize>(buf: &[u8], off: usize, len: usize) -> [u8; N] {
    buf[off..off + len].try_into().expect("static length")
}

// ---- package encryption -----------------------------------------------------

/// Encrypt the package in 4096-byte segments (each its own IV), prefixed by the
/// 8-byte little-endian original length — the `EncryptedPackage` stream body.
fn encrypt_package(package: &[u8], key: &[u8; 32], salt: &[u8; 16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(package.len() + 64);
    out.extend_from_slice(&(package.len() as u64).to_le_bytes());
    for (i, chunk) in package.chunks(SEGMENT).enumerate() {
        let iv = hash16(salt, &(i as u32).to_le_bytes());
        out.extend_from_slice(&aes_cbc(key, &iv, chunk));
    }
    out
}

// ---- password key derivation + verifier -------------------------------------

/// The verifier triplet + the wrapped package key (the `<p:encryptedKey>` attrs).
struct KeyMaterial {
    verifier_input: Vec<u8>,
    verifier_value: Vec<u8>,
    key_value: Vec<u8>,
}

/// Derive the password key and encrypt the verifier + the package key with it.
fn key_material(
    password: &str,
    salt: &[u8; 16],
    package_key: &[u8; 32],
    verifier: &[u8; 16],
) -> KeyMaterial {
    let hf = hash_password(password, salt);
    KeyMaterial {
        verifier_input: aes_cbc(&block_key(&hf, &BK_VERIFIER_INPUT), salt, verifier),
        verifier_value: aes_cbc(
            &block_key(&hf, &BK_VERIFIER_VALUE),
            salt,
            &sha512(&[verifier]),
        ),
        key_value: aes_cbc(&block_key(&hf, &BK_KEY_VALUE), salt, package_key),
    }
}

/// `H_final`: SHA-512 of the salted UTF-16LE password, spun `SPIN_COUNT` times.
fn hash_password(password: &str, salt: &[u8; 16]) -> [u8; 64] {
    let pw: Vec<u8> = password.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut h = sha512(&[salt, &pw]);
    for i in 0..SPIN_COUNT {
        h = sha512(&[&i.to_le_bytes(), &h]);
    }
    h
}

/// Derive a 32-byte AES key from the password hash and a block key.
fn block_key(hf: &[u8; 64], block: &[u8; 8]) -> [u8; 32] {
    slice(&sha512(&[hf, block]), 0, 32)
}

// ---- data integrity (HMAC) --------------------------------------------------

/// The encrypted HMAC key + value protecting the package against tampering.
struct DataIntegrity {
    hmac_key: Vec<u8>,
    hmac_value: Vec<u8>,
}

/// Encrypt a random HMAC key with the package key, then HMAC the encrypted
/// package and encrypt that too (both IVs derived from the key-data salt).
fn data_integrity(
    package_key: &[u8; 32],
    salt: &[u8; 16],
    hmac_key: &[u8; 64],
    enc_package: &[u8],
) -> DataIntegrity {
    let value = hmac_sha512(hmac_key, enc_package);
    DataIntegrity {
        hmac_key: aes_cbc(package_key, &hash16(salt, &BK_HMAC_KEY), hmac_key),
        hmac_value: aes_cbc(package_key, &hash16(salt, &BK_HMAC_VALUE), &value),
    }
}

// ---- crypto primitives ------------------------------------------------------

/// AES-256-CBC encrypt `data`, zero-padded to the block size.
fn aes_cbc(key: &[u8; 32], iv: &[u8; 16], data: &[u8]) -> Vec<u8> {
    let mut buf = data.to_vec();
    let pad = (16 - buf.len() % 16) % 16;
    buf.resize(buf.len() + pad, 0);
    let n = buf.len();
    Encryptor::<Aes256>::new(key.into(), iv.into())
        .encrypt_padded_mut::<NoPadding>(&mut buf, n)
        .expect("block-aligned");
    buf
}

/// SHA-512 of the concatenated parts.
fn sha512(parts: &[&[u8]]) -> [u8; 64] {
    let mut h = Sha512::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}

/// SHA-512 of `salt || block`, truncated to a 16-byte IV.
fn hash16(salt: &[u8; 16], block: &[u8]) -> [u8; 16] {
    slice(&sha512(&[salt, block]), 0, 16)
}

/// HMAC-SHA-512 of `data` under `key`.
fn hmac_sha512(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha512::new_from_slice(key).expect("any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

// ---- EncryptionInfo + CFB container -----------------------------------------

/// Build the `EncryptionInfo` stream: the agile version header + the XML descriptor.
fn encryption_info(
    key_salt: &[u8; 16],
    pw_salt: &[u8; 16],
    m: &KeyMaterial,
    di: &DataIntegrity,
) -> Vec<u8> {
    let xml = info_xml(key_salt, pw_salt, m, di);
    let mut out = Vec::with_capacity(xml.len() + 8);
    out.extend_from_slice(&[0x04, 0x00, 0x04, 0x00, 0x40, 0x00, 0x00, 0x00]);
    out.extend_from_slice(xml.as_bytes());
    out
}

/// The agile `<encryption>` XML descriptor (all binary fields base64-encoded).
fn info_xml(
    key_salt: &[u8; 16],
    pw_salt: &[u8; 16],
    m: &KeyMaterial,
    di: &DataIntegrity,
) -> String {
    let cipher = "blockSize=\"16\" keyBits=\"256\" hashSize=\"64\" \
                  cipherAlgorithm=\"AES\" cipherChaining=\"ChainingModeCBC\" hashAlgorithm=\"SHA512\"";
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
        <encryption xmlns=\"http://schemas.microsoft.com/office/2006/encryption\" \
        xmlns:p=\"http://schemas.microsoft.com/office/2006/keyEncryptor/password\">\
        <keyData saltSize=\"16\" {cipher} saltValue=\"{ks}\"/>\
        <dataIntegrity encryptedHmacKey=\"{hk}\" encryptedHmacValue=\"{hv}\"/>\
        <keyEncryptors><keyEncryptor uri=\"http://schemas.microsoft.com/office/2006/keyEncryptor/password\">\
        <p:encryptedKey spinCount=\"{spin}\" saltSize=\"16\" {cipher} saltValue=\"{ps}\" \
        encryptedVerifierHashInput=\"{vi}\" encryptedVerifierHashValue=\"{vv}\" encryptedKeyValue=\"{kv}\"/>\
        </keyEncryptor></keyEncryptors></encryption>",
        spin = SPIN_COUNT,
        ks = b64(key_salt),
        ps = b64(pw_salt),
        vi = b64(&m.verifier_input),
        vv = b64(&m.verifier_value),
        kv = b64(&m.key_value),
        hk = b64(&di.hmac_key),
        hv = b64(&di.hmac_value),
    )
}

/// Write the two streams into a CFB compound file and return its bytes.
fn build_cfb(info: &[u8], package: &[u8]) -> Result<Vec<u8>> {
    let mut comp = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new())).map_err(io_err)?;
    write_stream(&mut comp, "EncryptionInfo", info)?;
    write_stream(&mut comp, "EncryptedPackage", package)?;
    comp.flush().map_err(io_err)?;
    Ok(comp.into_inner().into_inner())
}

/// Create one named root stream and write its whole body.
fn write_stream(
    comp: &mut cfb::CompoundFile<std::io::Cursor<Vec<u8>>>,
    name: &str,
    body: &[u8],
) -> Result<()> {
    let mut stream = comp.create_stream(name).map_err(io_err)?;
    stream.write_all(body).map_err(io_err)?;
    stream.flush().map_err(io_err)
}

/// Map a CFB/IO error into a fatal encryption error.
fn io_err(e: std::io::Error) -> TurboXlsxError {
    enc_err(format!("cfb: {e}"))
}

// ---- base64 -----------------------------------------------------------------

const B64_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with `=` padding.
fn b64(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        out.push(B64_ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(B64_ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(b64_tail(chunk.len() > 1, (n >> 6) as usize & 63));
        out.push(b64_tail(chunk.len() > 2, n as usize & 63));
    }
    out
}

/// A tail base64 char, or `=` padding when the source byte is absent.
fn b64_tail(present: bool, idx: usize) -> char {
    if present {
        B64_ALPHABET[idx] as char
    } else {
        '='
    }
}
