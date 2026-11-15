//! Explicit, create-only agent onboarding. Existing agent instructions and MCP
//! configuration are never overwritten.
use excession::{Result, discovery, refuse};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

fn new_file(path: &Path, bytes: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        if parent.exists() {
            let meta = fs::symlink_metadata(parent)
                .map_err(|err| format!("inspect config parent {}: {err}", parent.display()))?;
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return refuse("agent config parent must be a real directory");
            }
        } else {
            fs::create_dir(parent).map_err(|err| {
                format!("create agent config directory {}: {err}", parent.display())
            })?;
        }
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| format!("create new agent config {}: {err}", path.display()))?;
    if let Err(err) = file
        .write_all(bytes.as_bytes())
        .and_then(|_| file.sync_all())
    {
        let _ = fs::remove_file(path);
        return refuse(format!("write agent config {}: {err}", path.display()));
    }
    Ok(())
}

fn existing(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => refuse(format!("inspect agent config {}: {err}", path.display())),
    }
}

fn configs(root: &Path, client: &str) -> Result<Vec<(&'static str, PathBuf, String)>> {
    let location = root
        .to_str()
        .ok_or("repository path cannot be represented as UTF-8 in MCP configuration")?;
    let args = ["mcp", "--repo", location];
    let mut result = Vec::new();
    if matches!(client, "codex" | "both") {
        // Local project config is loaded by Codex after the project is trusted.
        let path = root.join(".codex/config.toml");
        let toml = format!(
            "[mcp_servers.excession]\ncommand = \"excession\"\nargs = [\"mcp\", \"--repo\", {}]\n",
            serde_json::to_string(location).map_err(|err| err.to_string())?
        );
        result.push(("codex", path, toml));
    }
    if matches!(client, "claude" | "both") {
        let path = root.join(".mcp.json");
        let data = json!({"mcpServers":{"excession":{
            "type":"stdio","command":"excession","args":args
        }}});
        let config = serde_json::to_string_pretty(&data)
            .map_err(|err| format!("serialize Claude MCP config: {err}"))?;
        result.push(("claude", path, format!("{config}\n")));
    }
    Ok(result)
}

/// Read-only planning by default. --write creates missing project-level config
/// files but refuses to overwrite or silently merge existing configuration.
pub(super) fn init(repo: &Path, client: &str, write: bool) -> Result<Value> {
    if !matches!(client, "codex" | "claude" | "both") {
        return refuse("agent init --client must be codex, claude, or both");
    }
    let root = discovery::inspect(repo, "HEAD")?.root;
    let plans = configs(&root, client)?;
    let mut rows = Vec::new();
    // Check every target before any writes. A present file is preserved, not an error.
    let states = plans
        .iter()
        .map(|(_, path, _)| existing(path))
        .collect::<Result<Vec<_>>>()?;
    for ((name, path, content), present) in plans.iter().zip(states) {
        let status = if present {
            "EXISTING_CONFIG_PRESERVED"
        } else if write {
            new_file(path, content)?;
            "CREATED"
        } else {
            "PLANNED"
        };
        rows.push(json!({
            "client":name,
            "path":path.display().to_string(),
            "status":status,
            "proposed_content":content
        }));
    }
    let has_existing = rows
        .iter()
        .any(|r| r["status"] == "EXISTING_CONFIG_PRESERVED");
    Ok(json!({
        "schema":"excession.agent-integration.v1",
        "repository":root.display().to_string(),
        "status":if !write {"PLAN"} else if has_existing {"PARTIAL"} else {"CREATED"},
        "configurations":rows,
        "notes":[
            "Excession must be installed on PATH for the agent client.",
            "MCP runs against this local absolute repository path; do not commit machine-specific configuration without review.",
            "Existing configuration is preserved. Merge the proposed Excession entry manually when a file already exists.",
            "No AGENTS.md or CLAUDE.md instruction file is created or modified.",
            "MCP tools report Git-pinned evidence and do not execute project code or authorize merge."
        ]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn onboarding_is_explicit_and_preserves_preexisting_instructions_and_configs() {
        let root = std::env::temp_dir().join(format!(
            "excession-agent-init-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::create_dir_all(&root).unwrap();
        let git = |arguments: &[&str]| {
            let status = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(arguments)
                .status()
                .unwrap();
            assert!(status.success());
        };
        git(&["init", "-q"]);
        fs::write(root.join("AGENTS.md"), "Existing instructions\n").unwrap();
        git(&["add", "."]);
        git(&[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "-qm",
            "initial",
        ]);
        let plan = init(&root, "both", false).unwrap();
        assert_eq!(plan["status"], "PLAN");
        assert!(!root.join(".mcp.json").exists());
        let first = init(&root, "both", true).unwrap();
        assert_eq!(first["status"], "CREATED");
        assert!(root.join(".mcp.json").is_file());
        assert!(root.join(".codex/config.toml").is_file());
        let claude = fs::read_to_string(root.join(".mcp.json")).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&claude).unwrap()["mcpServers"]["excession"]["command"],
            "excession"
        );
        let again = init(&root, "both", true).unwrap();
        assert_eq!(again["status"], "PARTIAL");
        assert_eq!(fs::read_to_string(root.join(".mcp.json")).unwrap(), claude);
        assert_eq!(
            fs::read_to_string(root.join("AGENTS.md")).unwrap(),
            "Existing instructions\n"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
