//! DEX SHA-1 and Adler-32 integrity fields.

use sha1::{Digest, Sha1};

pub(super) fn signature(bytes: &[u8]) -> [u8; 20] {
    Sha1::digest(bytes).into()
}

pub(super) fn adler32(bytes: &[u8]) -> u32 {
    const MODULUS: u32 = 65_521;
    let mut first = 1u32;
    let mut second = 0u32;
    for chunk in bytes.chunks(5_552) {
        for byte in chunk {
            first += u32::from(*byte);
            second += first;
        }
        first %= MODULUS;
        second %= MODULUS;
    }
    (second << 16) | first
}
