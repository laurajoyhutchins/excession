//! Git subprocess adapter. No writes to source repositories; only reads.
use crate::{Result, refuse};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use sha1::{Digest, Sha1};

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

    /// Stream an entire Git blob through bounded buffers and verify the exact
    /// original Git object ID, including its length-prefixed object header.
    ///
    /// The callback receives monotonically increasing *byte* offsets. Its
    /// output is provisional until this method returns Ok: callers must stage
    /// changes in a transaction and commit only after final identity verification.
    /// A callback failure invalidates this batch reader rather than allowing
    /// a subsequent request against an undrained Git protocol response.
    pub fn stream_verified(
        &mut self,
        expected_oid: &str,
        chunk_bytes: usize,
        mut emit: impl FnMut(u64, &[u8]) -> Result<()>,
    ) -> Result<u64> {
        oid(expected_oid)?;
        if chunk_bytes == 0 {
            return refuse("Git blob stream chunk size must be positive");
        }
        let stdin = self.stdin.as_mut().ok_or("Git blob reader already closed")?;
        writeln!(stdin, "{expected_oid}").map_err(|e| format!("Git batch request: {e}"))?;
        stdin.flush().map_err(|e| format!("Git batch flush: {e}"))?;

        let mut header = String::new();
        if self.stdout.read_line(&mut header).map_err(|e| e.to_string())? == 0 {
            return refuse("Git batch ended before blob response");
        }
        let mut fields = header.trim_end_matches('\n').split_ascii_whitespace();
        if fields.next() != Some(expected_oid) || fields.next() != Some("blob") {
            return refuse("Git batch returned a missing or non-blob object");
        }
        let size = fields.next().ok_or("Git batch missing blob length")?
            .parse::<usize>().map_err(|_| "invalid Git batch blob length")?;
        if fields.next().is_some() {
            return refuse("unexpected Git batch header fields");
        }
        let mut hasher = Sha1::new();
        hasher.update(format!("blob {size}\0").as_bytes());
        let mut buffer = vec![0u8; chunk_bytes.min(64 * 1024)];
        let mut offset = 0usize;
        while offset < size {
            let len = (size - offset).min(buffer.len());
            self.stdout.read_exact(&mut buffer[..len])
                .map_err(|e| format!("truncated Git batch blob: {e}"))?;
            hasher.update(&buffer[..len]);
            if let Err(error) = emit(offset as u64, &buffer[..len]) {
                self.stdin.take();
                return Err(error);
            }
            offset += len;
        }
        let mut terminator = [0u8; 1];
        self.stdout.read_exact(&mut terminator)
            .map_err(|e| format!("missing Git batch blob separator: {e}"))?;
        if terminator != *b"\n" || crate::digest::hex(&hasher.finalize()) != expected_oid {
            return refuse("Git batch blob identity or framing mismatch");
        }
        Ok(size as u64)
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
            // Drain the oversized object without allocating it. A batch reader
            // must consume the entire framed response before the next OID.
            // Skipped payloads are never asserted as authenticated source.
            let drained = std::io::copy(
                &mut self.stdout.by_ref().take(size as u64),
                &mut std::io::sink(),
            )
            .map_err(|e| format!("draining oversized Git blob: {e}"))?;
            let mut terminator = [0u8; 1];
            self.stdout
                .read_exact(&mut terminator)
                .map_err(|e| format!("missing oversized Git blob separator: {e}"))?;
            if drained != size as u64 || terminator != *b"\n" {
                return refuse("invalid oversized Git batch framing");
            }
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

/// Read exactly one tracked tree entry without traversing the full repository.
pub fn entry_at(root: &Path, commit: &str, path: &str) -> Result<Entry> {
    oid(commit)?;
    validate_path(path)?;
    verify_repo(root)?;
    let literal = format!(":(literal){path}");
    let out = command(
        root,
        &["ls-tree", "-z", "--full-tree", commit, "--", &literal],
    )?;
    let record = out
        .strip_suffix(&[0])
        .ok_or("missing or unterminated Git entry")?;
    if record.contains(&0) {
        return refuse("ambiguous Git tree entry for exact path");
    }
    let tab = record
        .iter()
        .position(|byte| *byte == b'\t')
        .ok_or("invalid Git tree entry")?;
    let name = std::str::from_utf8(&record[tab + 1..]).map_err(|_| "invalid Git path UTF-8")?;
    if name != path {
        return refuse("Git tree path differs from requested exact path");
    }
    let header = std::str::from_utf8(&record[..tab]).map_err(|_| "invalid Git tree header")?;
    let parts = header.split_ascii_whitespace().collect::<Vec<_>>();
    if parts.len() != 3 {
        return refuse("invalid Git tree entry metadata");
    }
    if !matches!(
        (parts[0], parts[1]),
        ("100644" | "100755" | "120000", "blob")
    ) {
        return refuse("unsupported exact Git source entry type");
    }
    Ok(Entry {
        path: name.to_owned(),
        mode: parts[0].to_owned(),
        kind: parts[1].to_owned(),
        oid: oid(parts[2])?,
    })
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
    fn streamed_read_checks_full_blob_identity_with_bounded_chunks() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "excession-verified-stream-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&root).unwrap();
        assert!(Command::new("git").args(["init", "-q"]).arg(&root).status().unwrap().success());
        let source = "α🚀 planner query\n".repeat(100_000);
        std::fs::write(root.join("large.rs"), &source).unwrap();
        std::fs::write(root.join("empty.rs"), "").unwrap();
        let blob = text(&root, &["hash-object", "-w", "large.rs"]).unwrap();
        let empty = text(&root, &["hash-object", "-w", "empty.rs"]).unwrap();
        let mut reader = BlobReader::open(&root).unwrap();
        let mut reconstructed = Vec::new();
        let mut last = 0;
        let size = reader.stream_verified(&blob, 137, |offset, bytes| {
            assert!(bytes.len() <= 137);
            assert_eq!(offset, last);
            last += bytes.len() as u64;
            reconstructed.extend_from_slice(bytes);
            Ok(())
        }).unwrap();
        assert_eq!(size, source.len() as u64);
        assert_eq!(reconstructed, source.as_bytes());
        assert_eq!(reader.stream_verified(&empty, 4, |_, _| {
            panic!("empty Git blob must have no payload chunks")
        }).unwrap(), 0);
        assert_eq!(reader.read(&blob).unwrap(), source.as_bytes());
        assert!(reader.stream_verified(&blob, 0, |_, _| Ok(())).is_err());
        reader.finish().unwrap();
        let mut aborted = BlobReader::open(&root).unwrap();
        assert_eq!(aborted.stream_verified(&blob, 128, |_, _| Err("cancelled".into())), Err("cancelled".into()));
        assert!(aborted.read(&blob).is_err());
        drop(aborted);
        std::fs::remove_dir_all(root).unwrap();
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
        assert_eq!(
            reader.read_capped(&a, 1).unwrap_err(),
            "Git source blob exceeds byte limit"
        );
        // The next response must still begin with its own exact OID header.
        assert_eq!(reader.read(&b).unwrap(), second);
        reader.finish().unwrap();

        let mut reader = BlobReader::open(&root).unwrap();
        assert!(reader.read(&tree).is_err());
        drop(reader);
        let _ = std::fs::remove_dir_all(&root);
    }
}
