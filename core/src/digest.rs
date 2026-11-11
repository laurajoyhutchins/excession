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
    fn independent_padding_boundary_vectors() {
        // Generated independently with Python's hashlib. In particular, 56
        // and 120 bytes force an extra SHA padding block.
        let cases = [
            (
                1,
                "067d5096f219c64b53bb1c7d5e3754285b565a47",
                "e7cf46a078fed4fafd0b5e3aff144802b853f8ae459a4f0c14add3314b7cc3a6",
            ),
            (
                55,
                "c4622048cfef59b72875839ee7ae1cbcf55e7658",
                "2900465fcb533e05a158fd2b3be0e5e3b03740d83060aa3580e0d98a96bf2384",
            ),
            (
                56,
                "ddc12942656468475970fa4fa49161f52ed138e4",
                "31454ff48ef36af2f08fd511bdc37d9d5855ac23e992e5ff5445cb6b7674a674",
            ),
            (
                57,
                "c3299df7d6a56509d38a55d09ccf403d4a56d044",
                "bcc0a5d3791b985b7550e04ca660a6c63a589ba1edd2283c8e110e5b515df124",
            ),
            (
                63,
                "7f8c3fa49f1297bd8b9feb964b6b419987f9f0d1",
                "5f6401b96532c36de4e65beec0409b69b1d181864c8009b7a04f43e5d56350d1",
            ),
            (
                64,
                "a334b47180c61fd522f99905ec02c36f9e848211",
                "94eb5de4943613fd048dc93393ab06877405faa39c11f53e9386083339833e7e",
            ),
            (
                65,
                "dd27d9eb923d39687e10872c3e8133ba2f0a68a1",
                "fc518669b6eb4b4dd91827ecacef86689c725bd5bab888fd3b26dbb196eec954",
            ),
            (
                119,
                "bea949473b1ec34747ce121c3293624b5d9d8f84",
                "b0dc41b1a384e2f1203f0351b38fbeaafceef577ce1191d5bfc25da39f721eae",
            ),
            (
                120,
                "bf05266acd3ec21592b4d42aaea97fa6f3e51926",
                "5df24dd802ac26132ce608dcb5f09841eef039ee0f152acf98d26d17fe4e88e6",
            ),
            (
                121,
                "f7688ed0507cf4592f4c56e0f8c7c839569e3a2a",
                "5ed5a129bb49444fe2585d785920135a02f64350edf57c5f3ce7605b86f39039",
            ),
            (
                1000,
                "2c4169b2993df842a68ce33d5a1120339ab1057f",
                "57799de80e3dd6e2ac4d40c41a150d1662f7f87d0d994776a2fdc37c39b0ea4e",
            ),
            (
                4096,
                "c4a4f8cb5d332af2c2c970d28d45e66d22b12b82",
                "4e441a3533bb2c10cd5649981d395744213e09a336746b5a3458fee4057205ec",
            ),
        ];
        for (length, sha1_expected, sha256_expected) in cases {
            let data = (0..length).map(|i| (i * 37 + 11) as u8).collect::<Vec<_>>();
            assert_eq!(sha1(&data), sha1_expected, "SHA-1 length {length}");
            assert_eq!(sha256(&data), sha256_expected, "SHA-256 length {length}");
        }
    }

    #[test]
    fn malformed_hex_rejected() {
        assert!(unhex("0").is_err());
        assert!(unhex("zz").is_err());
        assert_eq!(unhex("00FF").expect("valid"), vec![0, 255]);
    }
}
