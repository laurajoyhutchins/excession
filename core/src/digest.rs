//! SHA-1 matches Git's SHA-1 object format, SHA-256 binds observations.
//! The process links against the system OpenSSL libcrypto.
use crate::{Result, refuse};
use std::ffi::c_uchar;

#[link(name = "crypto")]
unsafe extern "C" {
    fn SHA1(data: *const c_uchar, len: usize, out: *mut c_uchar) -> *mut c_uchar;
    fn SHA256(data: *const c_uchar, len: usize, out: *mut c_uchar) -> *mut c_uchar;
}

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
    let mut result = [0u8; 20];
    // SAFETY: `data` and `result` point to valid buffers of the required sizes.
    unsafe {
        SHA1(data.as_ptr(), data.len(), result.as_mut_ptr());
    }
    hex(&result)
}

pub fn sha256(data: &[u8]) -> String {
    let mut result = [0u8; 32];
    // SAFETY: `data` and `result` point to valid buffers of the required sizes.
    unsafe {
        SHA256(data.as_ptr(), data.len(), result.as_mut_ptr());
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
        assert_eq!(sha1(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(git_tree(b""), "4b825dc642cb6eb9a060e54bf8d69288fbee4904");
        assert_eq!(
            git_blob(b"test\n"),
            "9daeafb9864cf43055ae93beb0afd6c7d144bfa4"
        );
    }
    #[test]
    fn malformed_hex_rejected() {
        assert!(unhex("0").is_err());
        assert!(unhex("zz").is_err());
        assert_eq!(unhex("00FF").expect("valid"), vec![0, 255]);
    }
}
