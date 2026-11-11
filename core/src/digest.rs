//! Git SHA-1 object IDs and SHA-256 evidence digests.
//! Algorithms come from audited RustCrypto crates, not an in-house implementation.
//! Git SHA-1 is for legacy object identity only and is not collision resistant.
use crate::{Result, refuse};
use sha1::{Digest, Sha1};
use sha2::Sha256;

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

pub fn sha1(data: &[u8]) -> String {
    hex(&Sha1::digest(data))
}

pub fn sha256(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
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
