//! End-to-end MCP stdio acceptance with a real pinned Git repository.
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn stdio_agent_discovers_tools_and_authenticates_git_source() {
    let repo = std::env::temp_dir().join(format!(
        "excession-mcp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(repo.join("src")).unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init", "-q"]);
    fs::write(
        repo.join("src/lease.py"),
        "def renew_lease():\n    return True\n",
    )
    .unwrap();
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.invalid",
        "commit",
        "-qm",
        "source snapshot",
    ]);
    let commit = String::from_utf8(
        Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["rev-parse", "HEAD"])
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let commit = commit.trim().to_owned();
    // A working-tree edit must not silently replace the Git-pinned source.
    fs::write(
        repo.join("src/lease.py"),
        "def unrelated():\n    return False\n",
    )
    .unwrap();

    let requests = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2025-11-25","capabilities":{},
                "clientInfo":{"name":"acceptance","version":"0"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
            "name":"search_code","arguments":{"task":"renew lease","revision":"HEAD",
                "index_segments":256,"limit":4}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{
            "name":"get_context","arguments":{"focus":[]}}}),
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{
            "name":"shell","arguments":{}}}),
    ];
    let mut child = Command::new(env!("CARGO_BIN_EXE_excession"))
        .args(["mcp", "--repo", repo.to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let input = child.stdin.as_mut().unwrap();
        for request in requests {
            writeln!(input, "{request}").unwrap();
        }
    }
    // Close stdin to terminate the MCP server.
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let results = String::from_utf8(output.stdout).unwrap();
    let replies = results
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(replies.len(), 5, "notifications do not produce responses");
    assert_eq!(replies[0]["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(replies[1]["result"]["tools"].as_array().unwrap().len(), 6);
    assert_eq!(replies[2]["result"]["isError"], false);
    assert_eq!(
        replies[2]["result"]["structuredContent"]["source_commit"],
        commit
    );
    assert_eq!(
        replies[2]["result"]["structuredContent"]["candidates"][0]["source"]["path"],
        "src/lease.py"
    );
    assert_eq!(replies[3]["result"]["isError"], true);
    assert_eq!(replies[4]["error"]["code"], -32602);
    fs::remove_dir_all(repo).unwrap();
}
