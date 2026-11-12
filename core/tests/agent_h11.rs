//! Independent real-source acceptance for the agent CLI against pinned h11 Git history.
//! This checks source identity, executed bounded Python probes, and explicit
//! residual obligations, not application-level behavior or merge safety.

#[cfg(unix)]
#[test]
fn h11_agent_verification_is_source_pinned_and_never_auto_approves() {
    use serde_json::{Value, json};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Output};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn checked(cmd: &mut Command) -> Output {
        let result = cmd.output().expect("spawn CLI / native tool");
        assert!(
            result.status.success(),
            "{cmd:?} exited {:?}: {}",
            result.status.code(),
            String::from_utf8_lossy(&result.stderr)
        );
        result
    }

    fn git(repo: &Path, args: &[&str]) -> String {
        String::from_utf8(checked(Command::new("git").arg("-C").arg(repo).args(args)).stdout)
            .unwrap()
            .trim()
            .to_string()
    }

    fn agent(binary: &str, args: &[&str]) -> Value {
        let output = checked(Command::new(binary).args(args));
        serde_json::from_slice(&output.stdout).expect("machine-readable agent report")
    }

    fn observed_sha256(output: &Output) -> String {
        let mut log = output.stdout.clone();
        log.extend_from_slice(&output.stderr);
        excession::digest::sha256(&log)
    }

    let root = Scratch(std::env::temp_dir().join(format!(
        "excession-agent-h11-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    fs::create_dir_all(&root.0).unwrap();
    let repo = root.0.join("h11");
    let bundle = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/h11-crossrepo.bundle");
    checked(Command::new("git").arg("clone").arg("-q").arg(&bundle).arg(&repo));

    let base = git(&repo, &["rev-parse", "HEAD"]);
    let source = repo.join("h11/_connection.py");
    assert!(source.is_file(), "pinned h11 fixture must contain connection source");
    let binary = env!("CARGO_BIN_EXE_excession");
    let db = root.0.join("agent.sqlite");
    let db_str = db.to_str().unwrap();
    let context = agent(
        binary,
        &[
            "agent",
            "context",
            "--repo",
            repo.to_str().unwrap(),
            "--rev",
            &base,
            "--focus",
            "h11/_connection.py",
            "--task",
            "Review a connection module change",
            "--roots",
            "h11,tests",
            "--db",
            db_str,
            "--limit",
            "8",
        ],
    );
    assert_eq!(context["schema"], "excession.agent-context.v1");
    assert_eq!(context["source_commit"], base);
    assert_eq!(context["sources"][0]["path"], "h11/_connection.py");
    assert_eq!(context["coverage"]["status"], "PARTIAL");
    assert_eq!(context["safe_to_skip_unlisted_tests"], false);

    let original = fs::read_to_string(&source).unwrap();
    fs::write(&source, format!("{original}\n# independently reviewed h11 test edit\n")).unwrap();
    git(&repo, &["add", "h11/_connection.py"]);
    checked(
        Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args([
                "-c",
                "user.name=Excession h11 Acceptance",
                "-c",
                "user.email=excession@example.invalid",
                "commit",
                "-qm",
                "Pinned connection module change",
            ]),
    );
    let candidate = git(&repo, &["rev-parse", "HEAD"]);
    let tree = git(&repo, &["rev-parse", "HEAD^{tree}"]);
    assert_ne!(candidate, base);

    // These are actual, independently executed *narrow* Python source probes.
    // They cannot be substituted for a full h11 test suite.
    let build = checked(
        Command::new("python3")
            .args([
                "-c",
                "import pathlib,sys; p=pathlib.Path(sys.argv[1]); compile(p.read_text(), str(p), 'exec')",
            ])
            .arg(&source),
    );
    let test = checked(
        Command::new("python3")
            .args([
                "-c",
                "import ast,pathlib,sys; t=ast.parse(pathlib.Path(sys.argv[1]).read_text()); assert any(isinstance(x, ast.ClassDef) and x.name == 'Connection' for x in t.body)",
            ])
            .arg(&source),
    );
    let receipts = root.0.join("h11-receipts.json");
    let payload = json!({
        "schema":"excession.execution-receipts.v1",
        "receipts":[
            {
                "kind":"BUILD",
                "status":"PASS",
                "source_commit":candidate,
                "source_tree":tree,
                "producer":"h11-external-source-probe",
                "command":"python3 compile(h11/_connection.py)",
                "exit_code":0,
                "log_sha256":observed_sha256(&build)
            },
            {
                "kind":"TEST",
                "status":"PASS",
                "source_commit":candidate,
                "source_tree":tree,
                "producer":"h11-external-source-probe",
                "command":"python3 ast.parse + Connection class assertion",
                "exit_code":0,
                "log_sha256":observed_sha256(&test)
            }
        ]
    });
    fs::write(&receipts, serde_json::to_vec(&payload).unwrap()).unwrap();
    let report = agent(
        binary,
        &[
            "agent",
            "verify",
            "--repo",
            repo.to_str().unwrap(),
            "--base",
            &base,
            "--candidate",
            &candidate,
            "--roots",
            "h11,tests",
            "--db",
            db_str,
            "--receipts",
            receipts.to_str().unwrap(),
            "--limit",
            "8",
        ],
    );
    assert_eq!(report["schema"], "excession.agent-verification.v1");
    assert_eq!(report["base_commit"], base);
    assert_eq!(report["candidate_commit"], candidate);
    assert_eq!(report["candidate_tree"], tree);
    assert_eq!(report["change_count"], 1);
    assert_eq!(report["receipts"].as_array().unwrap().len(), 2);
    assert_eq!(report["replay"]["source"], "RECOMPUTED_FROM_PINNED_GIT");
    assert_eq!(report["status"], "OPEN_OBLIGATIONS");
    assert_eq!(report["ready_to_merge"], false);
    assert_eq!(report["safe_to_skip_unlisted_tests"], false);
    assert!(
        report["obligations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "RECEIPT_AUTHENTICITY_UNVERIFIED")
    );

    // A receipt for the base tree must never be accepted as candidate evidence.
    let mut stale = payload;
    stale["receipts"][0]["source_commit"] = base.into();
    fs::write(&receipts, serde_json::to_vec(&stale).unwrap()).unwrap();
    let rejected = Command::new(binary)
        .args([
            "agent",
            "verify",
            "--repo",
            repo.to_str().unwrap(),
            "--base",
            report["base_commit"].as_str().unwrap(),
            "--candidate",
            report["candidate_commit"].as_str().unwrap(),
            "--roots",
            "h11,tests",
            "--receipts",
            receipts.to_str().unwrap(),
            "--limit",
            "8",
        ])
        .output()
        .unwrap();
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty(), "refusal must not emit a success report");
}
