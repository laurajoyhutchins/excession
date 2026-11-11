//! Installed CLI smoke test: no dependence on the source checkout or shell utilities.
//! The interpreter-dependent portion runs when Python is available.
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn run(program: &str, args: &[&str], cwd: &Path) -> Output {
    let result = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("subprocess must launch");
    assert!(
        result.status.success(),
        "{program} {args:?} failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    result
}

#[test]
fn installed_binary_works_outside_build_tree_with_unicode_paths() {
    let binary = env!("CARGO_BIN_EXE_excession");
    let base = std::env::temp_dir().join(format!(
        "excession portable {} {} Ω",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let repo = base.join("source checkout");
    let working = base.join("independent working directory");
    fs::create_dir_all(repo.join("pkg")).expect("source fixture directory");
    fs::create_dir_all(&working).expect("independent working directory");

    let version = run(binary, &["--version"], &working);
    assert!(String::from_utf8_lossy(&version.stdout).contains("excession "));
    let help = run(binary, &["--help"], &working);
    assert!(String::from_utf8_lossy(&help.stdout).contains("index"));
    let index_help = run(binary, &["index", "--help"], &working);
    let index_usage = String::from_utf8_lossy(&index_help.stdout);
    assert!(index_usage.contains("--repo") && index_usage.contains("--db") && index_usage.contains("--name"));
    let doctor = run(binary, &["doctor", "--json"], &working);
    let diagnostics: serde_json::Value =
        serde_json::from_slice(&doctor.stdout).expect("valid doctor JSON");
    assert_eq!(diagnostics["schema"], "excession.capabilities.v1");
    assert_eq!(diagnostics["sqlite"]["status"], "BUNDLED");
    assert_eq!(diagnostics["tools"]["git"]["status"], "DETECTED");
    assert_eq!(diagnostics["required_ready"], true);

    if excession::adapter::python_command()
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("Python not installed: skipped optional indexed-language smoke");
        fs::remove_dir_all(&base).expect("remove empty smoke fixture");
        return;
    }

    fs::write(
        repo.join("pkg/__init__.py"),
        b"from .module import answer\n",
    )
    .unwrap();
    fs::write(
        repo.join("pkg/module.py"),
        b"def answer():\n    return 42\n",
    )
    .unwrap();
    let repo_arg = repo.to_str().expect("portable Unicode fixture");
    let cwd_arg = working.to_str().expect("portable Unicode fixture");
    run("git", &["init", "-q", repo_arg], &working);
    run("git", &["-C", repo_arg, "add", "--all"], &working);
    run(
        "git",
        &[
            "-C",
            repo_arg,
            "-c",
            "user.name=Excession Test",
            "-c",
            "user.email=portability@example.invalid",
            "commit",
            "-qm",
            "portable test fixture",
        ],
        &working,
    );

    let db = working.join("observations.sqlite");
    let db_arg = db.to_str().unwrap();
    let indexed = run(
        binary,
        &[
            "index", "--repo", repo_arg, "--db", db_arg, "--name", "portable", "--rev", "HEAD",
            "--roots", "pkg",
        ],
        &working,
    );
    assert!(String::from_utf8_lossy(&indexed.stdout).contains("GIT_TREE_INDEXED"));

    let view = run(
        binary,
        &[
            "view",
            "deps",
            "--db",
            db_arg,
            "--name",
            "portable",
            "--rev",
            "HEAD",
            "--seed",
            "pkg.module",
        ],
        &working,
    );
    let manifest = working.join("view.json");
    fs::write(&manifest, &view.stdout).expect("save manifest");
    let manifest_arg = manifest.to_str().unwrap();
    let replay = run(
        binary,
        &[
            "view",
            "replay",
            "deps",
            "--db",
            db_arg,
            "--name",
            "portable",
            "--rev",
            "HEAD",
            "--seed",
            "pkg.module",
            "--manifest",
            manifest_arg,
        ],
        &working,
    );
    assert!(String::from_utf8_lossy(&replay.stdout).contains("QUERY_REPLAY_MATCHED"));
    let listing = run(
        binary,
        &["view", "ls", "--repo", repo_arg, "--manifest", manifest_arg],
        &working,
    );
    assert!(String::from_utf8_lossy(&listing.stdout).contains("children"));
    assert!(Path::new(cwd_arg).is_absolute());

    fs::remove_dir_all(&base).expect("remove portability fixture");
}
