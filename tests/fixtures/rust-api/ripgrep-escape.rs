// Pinned excerpt from BurntSushi/ripgrep at 178cc027e283fd965cb72d0a7395b219eada170c
// Original: https://github.com/BurntSushi/ripgrep/blob/178cc027e283fd965cb72d0a7395b219eada170c/crates/globset/src/lib.rs
// Retains upstream's permissive MIT/Apache-2.0-or-Unlicense licensing.
pub fn escape(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            // note that ! does not need escaping because it is only special
            // inside brackets
            '?' | '*' | '[' | ']' | '{' | '}' => {
                escaped.push('[');
                escaped.push(c);
                escaped.push(']');
            }
            c => {
                escaped.push(c);
            }
        }
    }
    escaped
}
