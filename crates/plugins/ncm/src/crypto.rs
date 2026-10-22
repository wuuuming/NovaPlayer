//! NetEase's `weapi` request encryption, after NeteaseCloudMusicApi's `util/crypto.js`
//! (npm NeteaseCloudMusicApi 4.32.0): the params JSON is AES-128-CBC encrypted with a preset
//! key, the base64 of that again with a random 16-char key; the random key, reversed, is RSA
//! encrypted (no padding) with the service's public key as `encSecKey`.

use aes::Aes128;
use cipher::block_padding::Pkcs7;
use cipher::{BlockEncryptMut, KeyIvInit};
use num_bigint::BigUint;

const IV: &[u8; 16] = b"0102030405060708";
const PRESET_KEY: &[u8; 16] = b"0CoJUm6Qyw8W8jud";
/// The service's public key modulus (the PEM in crypto.js, SPKI-wrapped PKCS#1, e = 65537).
const MODULUS_HEX: &str = "00e0b509f6259df8642dbc35662901477df22677ec152b5ff68ace615bb7b725152b3ab17a876aea8a5aa76d2e417629ec4ee341f56135fccf695280104e0312ecbda92557c93870114af6c9d05c4f7f0c3685b7a46bee255932575cce10b424d813cfe4875d3e82047b97ddef52741d546b8e289dc6935b3ece0462db0a22b8e7";

#[allow(clippy::expect_used, reason = "both keys passed are 16 bytes: PRESET_KEY and a 16-character secret")]
fn aes_cbc(key: &[u8], text: &[u8]) -> Vec<u8> {
    cbc::Encryptor::<Aes128>::new_from_slices(key, IV).expect("16-byte key").encrypt_padded_vec_mut::<Pkcs7>(text)
}

/// RSA without padding: c = m^e mod n, as big-endian hex padded to the modulus length.
#[allow(clippy::expect_used, reason = "MODULUS_HEX is a constant that parses")]
fn rsa_no_pad(text: &str) -> String {
    let n = BigUint::parse_bytes(MODULUS_HEX.as_bytes(), 16).expect("modulus");
    let e = BigUint::from(65537u32);
    let m = BigUint::from_bytes_be(text.as_bytes());
    let c = m.modpow(&e, &n);
    let mut out = kit::util::hex(&c.to_bytes_be());
    while out.len() < 256 {
        out.insert(0, '0');
    }
    out
}

/// `(params, encSecKey)` for a POST to `/weapi/<path>`.
pub fn weapi(json_text: &str) -> (String, String) {
    let secret = kit::util::rand_base62(16);
    let first = kit::util::b64(&aes_cbc(PRESET_KEY, json_text.as_bytes()));
    let params = kit::util::b64(&aes_cbc(secret.as_bytes(), first.as_bytes()));
    let enc_sec_key = rsa_no_pad(&secret.chars().rev().collect::<String>());
    (params, enc_sec_key)
}

/// The anonymous-token username (module/register_anonimous.js): the device id, a space, and
/// the base64 of md5(deviceId XOR "3go8&$8*3*3h0k(2)2"), all base64'd together.
pub fn anonymous_username(device_id: &str) -> String {
    const XOR_KEY: &[u8] = b"3go8&$8*3*3h0k(2)2";
    let xored: Vec<u8> = device_id.bytes().enumerate().map(|(i, b)| b ^ XOR_KEY[i % XOR_KEY.len()]).collect();
    kit::util::b64(format!("{device_id} {}", kit::util::b64(&md5::compute(&xored).0)).as_bytes())
}
