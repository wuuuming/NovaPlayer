//! QQ web CSRF hash and encrypted lyric decoding. Format reference: QQMusicApi /
//! folia-major qrcDecrypt.ts: triple-DES ECB, then zlib (not raw UTF-8).

/// Tencent's hash33 (g_tk / ptqrtoken): h = h*33 + c, masked to 31 bits.
pub fn hash33(s: &str, seed: u64) -> u64 {
    let mut h = seed;
    for c in s.chars() {
        h = h.wrapping_mul(33).wrapping_add(c as u64) & 0x7fffffff;
    }
    h & 0x7fffffff
}

const QRC_KEY: &[u8; 24] = b"!@#)(*$%123ZXC!@!@#)(NHL";
const MAX_LYRIC_BYTES: usize = 2 * 1024 * 1024;

pub fn qrc_decrypt(hex_text: &str) -> Option<String> {
    use cipher::{BlockDecrypt, KeyInit};
    let hex_text = hex_text.trim();
    if hex_text.is_empty() || hex_text.len() > MAX_LYRIC_BYTES * 2 {
        return None;
    }
    let mut data = kit::util::unhex(hex_text)?;
    // Ciphertext is block-aligned; padding incomplete ciphertext invents bytes.
    if !data.len().is_multiple_of(8) {
        return None;
    }
    let cipher = des::TdesEde3::new_from_slice(QRC_KEY).ok()?;
    for block in data.chunks_mut(8) {
        cipher.decrypt_block(cipher::generic_array::GenericArray::from_mut_slice(block));
    }
    inflate_lyric(&data)
}

fn inflate_lyric(data: &[u8]) -> Option<String> {
    let mut decoder = flate2::Decompress::new(true);
    let mut text = vec![0; MAX_LYRIC_BYTES + 1];
    let status = decoder.decompress(data, &mut text, flate2::FlushDecompress::Finish).ok()?;
    let len = decoder.total_out() as usize;
    if status != flate2::Status::StreamEnd || len > MAX_LYRIC_BYTES {
        return None;
    }
    text.truncate(len);
    String::from_utf8(text).ok()
}
