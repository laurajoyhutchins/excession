//! SHA-1 matches Git's SHA-1 object format; SHA-256 binds observations.
//! Both digests are implemented in portable Rust without a system crypto library.
//! These are legacy/source-identity primitives, not password hashing or signatures.
use crate::{Result, refuse};

pub fn hex(data: &[u8]) -> String {
    const CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(data.len() * 2);
    for &c in data {
        s.push(char::from(CHARS[(c >> 4) as usize]));
        s.push(char::from(CHARS[(c & 15) as usize]));
    }
    s
}

pub fn unhex(s: &str) -> Result<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return refuse("odd-length hexadecimal input");
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for chars in s.as_bytes().as_chunks::<2>().0 {
        fn n(c: u8) -> Option<u8> {
            match c {
                b'0'..=b'9' => Some(c - b'0'),
                b'a'..=b'f' => Some(c - b'a' + 10),
                b'A'..=b'F' => Some(c - b'A' + 10),
                _ => None,
            }
        }
        out.push(
            (n(chars[0]).ok_or("invalid hex digit")? << 4)
                | n(chars[1]).ok_or("invalid hex digit")?,
        );
    }
    Ok(out)
}

// SHA-1 and SHA-256 use the same 512-bit Merkle-Damgård padding. The
// original length is encoded modulo 2^64, as specified by both algorithms.
fn padded(data: &[u8]) -> Vec<u8> {
    let mut message = Vec::with_capacity(data.len().saturating_add(72));
    message.extend_from_slice(data);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&(data.len() as u64).wrapping_mul(8).to_be_bytes());
    message
}

pub fn sha1(data: &[u8]) -> String {
    let mut state = [
        0x6745_2301_u32,
        0xefcd_ab89,
        0x98ba_dcfe,
        0x1032_5476,
        0xc3d2_e1f0,
    ];
    for chunk in padded(data).as_chunks::<64>().0 {
        let mut words = [0_u32; 80];
        for (i, word) in words[..16].iter_mut().enumerate() {
            let start = i * 4;
            *word = u32::from_be_bytes(chunk[start..start + 4].try_into().expect("four bytes"));
        }
        for i in 16..80 {
            words[i] = (words[i - 3] ^ words[i - 8] ^ words[i - 14] ^ words[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (i, &word) in words.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5a82_7999),
                20..=39 => (b ^ c ^ d, 0x6ed9_eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1b_bcdc),
                _ => (b ^ c ^ d, 0xca62_c1d6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = next;
        }
        for (destination, addition) in state.iter_mut().zip([a, b, c, d, e]) {
            *destination = destination.wrapping_add(addition);
        }
    }
    let mut result = [0_u8; 20];
    for (i, word) in state.iter().enumerate() {
        result[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
    }
    hex(&result)
}

pub fn sha256(data: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    for chunk in padded(data).as_chunks::<64>().0 {
        let mut words = [0_u32; 64];
        for (i, word) in words[..16].iter_mut().enumerate() {
            let start = i * 4;
            *word = u32::from_be_bytes(chunk[start..start + 4].try_into().expect("four bytes"));
        }
        for i in 16..64 {
            let s0 = words[i - 15].rotate_right(7)
                ^ words[i - 15].rotate_right(18)
                ^ (words[i - 15] >> 3);
            let s1 = words[i - 2].rotate_right(17)
                ^ words[i - 2].rotate_right(19)
                ^ (words[i - 2] >> 10);
            words[i] = words[i - 16]
                .wrapping_add(s0)
                .wrapping_add(words[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for (i, &word) in words.iter().enumerate() {
            let big_s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choose = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(big_s1)
                .wrapping_add(choose)
                .wrapping_add(K[i])
                .wrapping_add(word);
            let big_s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = big_s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (destination, addition) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *destination = destination.wrapping_add(addition);
        }
    }
    let mut result = [0_u8; 32];
    for (i, word) in state.iter().enumerate() {
        result[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
    }
    hex(&result)
}

pub fn git_blob(data: &[u8]) -> String {
    let mut payload = format!("blob {}\0", data.len()).into_bytes();
    payload.extend_from_slice(data);
    sha1(&payload)
}

pub fn git_tree(data: &[u8]) -> String {
    let mut payload = format!("tree {}\0", data.len()).into_bytes();
    payload.extend_from_slice(data);
    sha1(&payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_hashes() {
        assert_eq!(sha1(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(sha1(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        assert_eq!(
            sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(git_tree(b""), "4b825dc642cb6eb9a060e54bf8d69288fbee4904");
        assert_eq!(
            git_blob(b"test\n"),
            "9daeafb9864cf43055ae93beb0afd6c7d144bfa4"
        );
    }

    #[test]
    fn million_byte_vectors_cover_many_blocks() {
        let message = vec![b'a'; 1_000_000];
        assert_eq!(sha1(&message), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
        assert_eq!(
            sha256(&message),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn malformed_hex_rejected() {
        assert!(unhex("0").is_err());
        assert!(unhex("zz").is_err());
        assert_eq!(unhex("00FF").expect("valid"), vec![0, 255]);
    }
}
