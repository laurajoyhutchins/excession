//! Git subprocess adapter. No writes to source repositories; only reads.
use crate::{Result, refuse};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: String,
    pub mode: String,
    pub kind: String,
    pub oid: String,
}

pub fn command(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let t0 = std::time::Instant::now();
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| format!("git launch: {e}"))?;
    if std::env::var("EXCESSION_TIMING").ok().as_deref() == Some("1") {
        eprintln!(
            "TIMING git {:.2}ms {}",
            t0.elapsed().as_secs_f64() * 1000.,
            args.join(" ")
        );
    }
    if !output.status.success() {
        return refuse(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(output.stdout)
}

pub fn text(root: &Path, args: &[&str]) -> Result<String> {
    let out = command(root, args)?;
    Ok(String::from_utf8(out)
        .map_err(|_| "Git produced non-UTF-8 scalar")?
        .trim()
        .to_owned())
}

pub fn oid(s: &str) -> Result<String> {
    if s.len() != 40
        || !s.bytes().all(|b| b.is_ascii_hexdigit())
        || s.chars().any(|c| c.is_ascii_uppercase())
    {
        return refuse(format!(
            "unsupported Git OID (requires lowercase SHA-1): {s}"
        ));
    }
    Ok(s.to_owned())
}

pub fn verify_repo(root: &Path) -> Result<()> {
    if !root.join(".git").exists() {
        return refuse(format!("not an existing checkout: {}", root.display()));
    }
    let format = text(root, &["rev-parse", "--show-object-format"])?;
    if format != "sha1" {
        return refuse(format!("unsupported Git object format: {format}"));
    }
    Ok(())
}

pub fn clean(root: &Path) -> Result<()> {
    verify_repo(root)?;
    if !command(root, &["status", "--porcelain", "--untracked-files=all"])?.is_empty() {
        return refuse(format!("dirty checkout: {}", root.display()));
    }
    Ok(())
}

pub fn rev(root: &Path, refspec: &str) -> Result<String> {
    oid(&text(root, &["rev-parse", "--verify", refspec])?)
}

pub fn blob(root: &Path, refspec: &str, path: &str) -> Result<String> {
    validate_path(path)?;
    rev(root, &format!("{refspec}:{path}"))
}

/// One authenticated Git subprocess for a sequence of source blobs.
///
/// Each request is answered before the next is submitted, so large blobs
/// cannot deadlock a full stdin/stdout pipe. No shell or external crate is
/// involved; object type, identity, length, terminator and content are checked.
pub struct BlobReader {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    finished: bool,
}

impl BlobReader {
    pub fn open(root: &Path) -> Result<Self> {
        verify_repo(root)?;
        let mut child = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("git cat-file --batch launch: {e}"))?;
        let stdin = child.stdin.take().ok_or("missing Git batch stdin")?;
        let stdout = child.stdout.take().ok_or("missing Git batch stdout")?;
        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
            finished: false,
        })
    }

    pub fn read(&mut self, expected_oid: &str) -> Result<Vec<u8>> {
        self.read_capped(expected_oid, usize::MAX)
    }

    /// Reject oversized blobs before allocating their payload.
    pub fn read_capped(&mut self, expected_oid: &str, max_bytes: usize) -> Result<Vec<u8>> {
        oid(expected_oid)?;
        let stdin = self
            .stdin
            .as_mut()
            .ok_or("Git blob reader already closed")?;
        writeln!(stdin, "{expected_oid}").map_err(|e| format!("Git batch request: {e}"))?;
        stdin.flush().map_err(|e| format!("Git batch flush: {e}"))?;

        let mut header = String::new();
        if self
            .stdout
            .read_line(&mut header)
            .map_err(|e| e.to_string())?
            == 0
        {
            return refuse("Git batch ended before blob response");
        }
        let mut fields = header.trim_end_matches('\n').split_ascii_whitespace();
        if fields.next() != Some(expected_oid) || fields.next() != Some("blob") {
            return refuse("Git batch returned a missing or non-blob object");
        }
        let size = fields
            .next()
            .ok_or("Git batch missing blob length")?
            .parse::<usize>()
            .map_err(|_| "invalid Git batch blob length")?;
        if fields.next().is_some() {
            return refuse("unexpected Git batch header fields");
        }
        if size > max_bytes {
            return refuse("Git source blob exceeds byte limit");
        }

        let mut data = vec![0_u8; size];
        self.stdout
            .read_exact(&mut data)
            .map_err(|e| format!("truncated Git batch blob: {e}"))?;
        let mut terminator = [0_u8; 1];
        self.stdout
            .read_exact(&mut terminator)
            .map_err(|e| format!("missing Git batch blob separator: {e}"))?;
        if terminator != *b"\n" || crate::digest::git_blob(&data) != expected_oid {
            return refuse("Git batch blob identity or framing mismatch");
        }
        Ok(data)
    }

    pub fn finish(mut self) -> Result<()> {
        self.stdin.take(); // EOF lets Git exit normally after its final response.
        let status = self
            .child
            .wait()
            .map_err(|e| format!("Git batch wait: {e}"))?;
        self.finished = true;
        if !status.success() {
            return refuse(format!("git cat-file --batch exited with {status}"));
        }
        Ok(())
    }
}

impl Drop for BlobReader {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub fn changed(root: &Path, old: &str, new: &str) -> Result<Vec<String>> {
    let out = command(root, &["diff", "--name-only", "-z", old, new])?;
    if out.last() == Some(&0) {
    } else if !out.is_empty() {
        return refuse("unterminated Git diff entry");
    }
    let mut result = Vec::new();
    for field in out.split(|b| *b == 0).filter(|x| !x.is_empty()) {
        let path = std::str::from_utf8(field).map_err(|_| "non-UTF8 changed Git filename")?;
        validate_path(path)?;
        result.push(path.to_owned());
    }
    result.sort();
    Ok(result)
}

pub fn validate_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\0')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return refuse(format!("unsafe Git path: {path:?}"));
    }
    Ok(())
}

pub fn entries(root: &Path, commit: &str) -> Result<Vec<Entry>> {
    oid(commit)?;
    verify_repo(root)?;
    let stdout = command(root, &["ls-tree", "-rtz", "--full-tree", commit])?;
    if stdout.last() != Some(&0) && !stdout.is_empty() {
        return refuse("unterminated git ls-tree result");
    }
    let mut out = Vec::new();
    for line in stdout.split(|b| *b == 0).filter(|s| !s.is_empty()) {
        let tab = line
            .iter()
            .position(|b| *b == b'\t')
            .ok_or("invalid ls-tree line")?;
        let header = std::str::from_utf8(&line[..tab]).map_err(|_| "invalid ls-tree header")?;
        let parts = header.split(' ').collect::<Vec<_>>();
        if parts.len() != 3 {
            return refuse("invalid ls-tree metadata");
        }
        let path = std::str::from_utf8(&line[tab + 1..])
            .map_err(|_| "non-UTF8 Git filename unsupported")?
            .to_owned();
        validate_path(&path)?;
        let (mode, kind, oidvalue) = (parts[0], parts[1], parts[2]);
        if !matches!(
            (kind, mode),
            ("blob", "100644" | "100755" | "120000") | ("tree", "040000") | ("commit", "160000")
        ) {
            return refuse(format!("unsupported Git entry type/mode {kind}/{mode}"));
        }
        out.push(Entry {
            path,
            mode: mode.into(),
            kind: kind.into(),
            oid: oid(oidvalue)?,
        });
    }
    let mut ordered = out.iter().map(|e| e.path.as_str()).collect::<Vec<_>>();
    ordered.sort();
    if ordered.windows(2).any(|two| two[0] == two[1]) {
        return refuse("duplicate Git path");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_refuse_traversal() {
        for bad in ["../x", "a/../b", "/tmp", "x//y", "a\\b", "a/./b"] {
            assert!(validate_path(bad).is_err());
        }
        assert!(validate_path("src/bin/hello.rs").is_ok());
    }
    #[test]
    fn oids_refuse_non_sha1() {
        assert!(oid("a").is_err());
        assert!(oid(&"F".repeat(40)).is_err());
        assert!(oid(&"a".repeat(64)).is_err());
    }
    #[test]
    fn streamed_blobs_preserve_identity_and_refuse_wrong_object_types() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "excession-blob-reader-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .arg(&root)
                .status()
                .unwrap()
                .success()
        );

        let first = b"first\n";
        let second = b"\0\xff\nsecond";
        std::fs::write(root.join("first.bin"), first).unwrap();
        std::fs::write(root.join("second.bin"), second).unwrap();
        let a = text(&root, &["hash-object", "-w", "first.bin"]).unwrap();
        let b = text(&root, &["hash-object", "-w", "second.bin"]).unwrap();
        let tree = text(&root, &["hash-object", "-w", "-t", "tree", "--stdin"]).unwrap();

        let mut reader = BlobReader::open(&root).unwrap();
        assert_eq!(reader.read(&a).unwrap(), first);
        assert_eq!(reader.read(&b).unwrap(), second);
        assert_eq!(reader.read(&a).unwrap(), first);
        reader.finish().unwrap();

        let mut reader = BlobReader::open(&root).unwrap();
        assert!(reader.read(&tree).is_err());
        drop(reader);
        let _ = std::fs::remove_dir_all(&root);
    }
}
