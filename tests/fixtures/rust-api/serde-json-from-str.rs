// Pinned excerpt from serde-rs/json at 8c535a260ce561eaab4111aafc29038eb9f77ff3
// Original: https://github.com/serde-rs/json/blob/8c535a260ce561eaab4111aafc29038eb9f77ff3/src/de.rs
// SPDX-License-Identifier: MIT OR Apache-2.0 (upstream project)
pub fn from_str<'a, T>(s: &'a str) -> Result<T>
where
    T: de::Deserialize<'a>,
{
    from_trait(read::StrRead::new(s))
}
