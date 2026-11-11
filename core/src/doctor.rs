//! Bounded local capability discovery. No project code, downloads or network calls.
use crate::adapter;
use crate::json::{Value, obj};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const PROBE_LIMIT: Duration = Duration::from_secs(2);

struct Probe {
    json: Value,
    available: bool,
}

fn probe(name: &str, required: bool, mut command: Command, args: &[&str]) -> Probe {
    let executable = command.get_program().to_string_lossy().into_owned();
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let (available, version, status) = match command.spawn() {
        Err(_) => (false, Value::Null, "UNAVAILABLE"),
        Ok(mut child) => {
            let start = Instant::now();
            let mut finished = false;
            while start.elapsed() < PROBE_LIMIT {
                match child.try_wait() {
                    Ok(Some(_)) => {
                        finished = true;
                        break;
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                    Err(_) => break,
                }
            }
            if !finished {
                let _ = child.kill();
            }
            match child.wait_with_output() {
                Ok(output) if finished && output.status.success() => {
                    let raw = if output.stdout.is_empty() {
                        output.stderr.as_slice()
                    } else {
                        output.stdout.as_slice()
                    };
                    let first = String::from_utf8_lossy(raw);
                    let value = first.lines().next().unwrap_or("").trim();
                    // Tool output is observational, never a trusted capability guarantee.
                    (true, Value::string(value.chars().take(180).collect::<String>()), "DETECTED")
                }
                _ if finished => (false, Value::Null, "FAILED"),
                _ => (false, Value::Null, "TIMED_OUT"),
            }
        }
    };
    Probe {
        json: obj(&[
            ("name", Value::string(name)),
            ("executable", Value::string(executable)),
            ("required", Value::Bool(required)),
            ("status", Value::string(status)),
            ("version", version),
        ]),
        available,
    }
}

pub fn report() -> Value {
    let git = probe("git", true, Command::new("git"), &["--version"]);
    let python = probe("python", false, adapter::python_command(), &["--version"]);
    let node = probe("node", false, adapter::node_command(), &["--version"]);
    let go = probe("go", false, Command::new("go"), &["version"]);
    let codeql = probe("codeql", false, Command::new("codeql"), &["version"]);
    let semgrep = probe("semgrep", false, Command::new("semgrep"), &["--version"]);
    let rust_analyzer = probe(
        "rust-analyzer", false, Command::new("rust-analyzer"), &["--version"]
    );
    let typescript = match adapter::typescript_file() {
        Ok(path) => obj(&[
            ("path", Value::string(path.to_string_lossy())),
            ("status", Value::string("PRESENT_VERSION_ENFORCED_AT_USE")),
            ("expected_version", Value::string("5.8.3")),
        ]),
        Err(_) => obj(&[
            ("path", Value::Null),
            ("status", Value::string("UNAVAILABLE")),
            ("expected_version", Value::string("5.8.3")),
        ]),
    };
    let has_ts = typescript.get("path").ok().is_some_and(|v| *v != Value::Null);
    let mandatory_ready = git.available;
    let python_ready = python.available;
    let node_ready = node.available;
    obj(&[
        ("schema", Value::string("excession.capabilities.v1")),
        ("excession", Value::string(env!("CARGO_PKG_VERSION"))),
        ("platform", Value::string(format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH))),
        ("required_ready", Value::Bool(mandatory_ready)),
        ("sqlite", obj(&[
            ("status", Value::string("BUNDLED")),
            ("version", Value::string(crate::sqlite::Db::version())),
        ])),
        ("tools", obj(&[
            ("git", git.json),
            ("python", python.json),
            ("node", node.json),
            ("go", go.json),
            ("codeql", codeql.json),
            ("semgrep", semgrep.json),
            ("rust_analyzer", rust_analyzer.json),
            ("typescript", typescript),
        ])),
        ("capabilities", obj(&[
            ("git_views", Value::Bool(mandatory_ready)),
            ("python_index", Value::Bool(mandatory_ready && python_ready)),
            ("javascript_index", Value::Bool(mandatory_ready && node_ready && has_ts)),
            ("syntax_bridge", Value::string(if python_ready {
                "PYTHON_FOUND_GRAMMARS_NOT_CHECKED"
            } else {
                "PYTHON_UNAVAILABLE"
            })),
            ("native_toolchains", Value::string(
                "PER_LANGUAGE_CONFIGURATION_REQUIRED"
            )),
        ])),
    ])
}

pub fn format_human(report: &Value) -> crate::Result<String> {
    let mut lines = Vec::new();
    lines.push(format!(
        "Excession {} on {}",
        report.get("excession")?.str()?,
        report.get("platform")?.str()?
    ));
    lines.push(format!(
        "SQLite {} (bundled), no external SQLite installation",
        report.get("sqlite")?.get("version")?.str()?
    ));
    for name in [
        "git", "python", "node", "go", "rust_analyzer", "codeql", "semgrep",
    ] {
        let record = report.get("tools")?.get(name)?;
        let status = record.get("status")?.str()?;
        let version = match record.get("version")? {
            Value::String(value) if !value.is_empty() => format!(" ({value})"),
            _ => String::new(),
        };
        lines.push(format!("  {name}: {status}{version}"));
    }
    lines.push(format!(
        "  typescript: {}",
        report.get("tools")?.get("typescript")?.get("status")?.str()?
    ));
    lines.push("Missing optional tools disable only their corresponding adapters.".to_owned());
    lines.push(
        "Discovery checks installed tools, not the correctness of project-specific analysis."
            .to_owned(),
    );
    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_schema_is_explicit_about_unchecked_grammars() {
        let report = report();
        assert_eq!(report.get("schema").unwrap().str().unwrap(), "excession.capabilities.v1");
        assert_eq!(report.get("sqlite").unwrap().get("status").unwrap().str().unwrap(), "BUNDLED");
        let bridge = report.get("capabilities").unwrap().get("syntax_bridge").unwrap().str().unwrap();
        assert!(bridge.contains("GRAMMARS_NOT_CHECKED") || bridge == "PYTHON_UNAVAILABLE");
    }
}
