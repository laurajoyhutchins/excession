//! Explicit trusted-local orchestration for installed scanner engines. No shell,
//! source checkout mutation, implicit downloads, or claim of sandbox isolation.
use super::Options;
use excession::json::Value;
use excession::sqlite::Db;
use excession::{Result, git, refuse, security_ingest};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn outside_source(repo: &Path, output: &Path) -> Result<()> {
    if !output.is_absolute() {
        return refuse("scanner evidence output paths must be absolute");
    }
    let parent = output
        .parent()
        .ok_or("output has no parent")?
        .canonicalize()
        .map_err(|e| format!("output directory unavailable: {e}"))?;
    if parent.starts_with(repo) {
        return refuse("scanner evidence must be outside source checkout");
    }
    if let Ok(meta) = fs::symlink_metadata(output) {
        if meta.file_type().is_symlink() {
            return refuse("scanner evidence path must not be a symlink");
        }
        let actual = output.canonicalize().map_err(|e| e.to_string())?;
        if actual.starts_with(repo) {
            return refuse("scanner evidence resolves inside source checkout");
        }
    }
    Ok(())
}

pub(super) fn run(opt: &Options) -> Result<Value> {
    if opt.get("trusted-local") != "true" {
        return refuse("scanner execution requires --trusted-local true; no sandbox is provided");
    }
    let repo = opt.path("repo").canonicalize().map_err(|e| e.to_string())?;
    git::verify_repo(&repo)?;
    git::clean(&repo)?;
    let commit = git::rev(&repo, "HEAD")?;
    let report = opt.path("report");
    let evidence = opt.path("db");
    outside_source(&repo, report)?;
    outside_source(&repo, evidence)?;
    if fs::symlink_metadata(report).is_ok() {
        return refuse("scanner report already exists; refusing overwrite");
    }
    let config = opt.path("config");
    if !config.is_file() {
        return refuse("scanner requires an existing, reviewed local rule/query file");
    }
    let config = config.canonicalize().map_err(|e| e.to_string())?;
    let seconds = opt
        .get_or("timeout-seconds", "300")
        .parse::<u64>()
        .map_err(|_| "--timeout-seconds must be 1..300".to_owned())?;
    if !(1..=300).contains(&seconds) {
        return refuse("--timeout-seconds must be 1..300");
    }
    let engine = opt.get("engine");
    let executable = opt.get_or("scanner", engine);
    let mut command = Command::new(executable);
    match engine {
        "semgrep" => {
            if opt.0.contains_key("codeql-db") {
                return refuse("--codeql-db applies only to CodeQL");
            }
            command
                .args(["scan", "--metrics=off", "--json", "--config"])
                .arg(&config)
                .arg("--output")
                .arg(report)
                .arg(".")
                .current_dir(&repo)
                .env("SEMGREP_SEND_METRICS", "off");
        }
        "codeql" => {
            let Some(database) = opt.0.get("codeql-db") else {
                return refuse("CodeQL requires --codeql-db pointing at a prebuilt database");
            };
            let database = PathBuf::from(database);
            if !database.exists() {
                return refuse("CodeQL database does not exist");
            }
            command
                .args(["database", "analyze"])
                .arg(database.canonicalize().map_err(|e| e.to_string())?)
                .arg(&config)
                .arg("--format=sarif-latest")
                .arg(format!("--output={}", report.display()));
        }
        _ => return refuse("scanner engine must be codeql or semgrep"),
    }
    command.stdout(Stdio::null()).stderr(Stdio::inherit());
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start {engine} scanner: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    let exit_status = loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return refuse(format!("{engine} scanner exceeded {seconds}s timeout"));
            }
            None => thread::sleep(Duration::from_millis(50)),
        }
    };
    if !exit_status.success() {
        return refuse(format!("{engine} scanner failed: {exit_status}"));
    }
    if !report.is_file() || report.metadata().map_err(|e| e.to_string())?.len() > 16 * 1024 * 1024 {
        return refuse("scanner produced no report or a report larger than 16 MiB");
    }
    git::clean(&repo)?;
    if git::rev(&repo, "HEAD")? != commit {
        return refuse("source HEAD changed during scanning");
    }
    let db = Db::open(evidence)?;
    let initial = security_ingest::execute(&repo, report, &db, engine, false)?;
    let audited = security_ingest::execute(&repo, report, &db, engine, true)?;
    if initial.get("run_id")?.str()? != audited.get("run_id")?.str()? {
        return refuse("scan audit returned a different run identity");
    }
    Ok(audited)
}
