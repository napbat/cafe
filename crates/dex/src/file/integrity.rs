//! DEX SHA-1 and Adler-32 integrity fields.

use sha1::{Digest, Sha1};

use super::header::SIGNATURE_SIZE;

const ADLER_MODULUS: u32 = 65_521;
const ADLER_CHUNK_SIZE: usize = 5_552;
const ADLER_HIGH_COMPONENT_SHIFT: u32 = u16::BITS;
const ADLER_INITIAL_FIRST_COMPONENT: u32 = 1;
const ADLER_INITIAL_SECOND_COMPONENT: u32 = 0;

pub(super) fn signature(bytes: &[u8]) -> [u8; SIGNATURE_SIZE] {
    Sha1::digest(bytes).into()
}

pub(super) fn adler32(bytes: &[u8]) -> u32 {
    let mut first = ADLER_INITIAL_FIRST_COMPONENT;
    let mut second = ADLER_INITIAL_SECOND_COMPONENT;
    for chunk in bytes.chunks(ADLER_CHUNK_SIZE) {
        for byte in chunk {
            first += u32::from(*byte);
            second += first;
        }
        first %= ADLER_MODULUS;
        second %= ADLER_MODULUS;
    }
    (second << ADLER_HIGH_COMPONENT_SHIFT) | first
}
