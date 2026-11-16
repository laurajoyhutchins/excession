//! End-to-end MCP stdio acceptance with a real pinned Git repository.
use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn rpc(input: &mut ChildStdin, output: &mut BufReader<ChildStdout>, request: Value) -> Value {
    writeln!(input, "{request}").unwrap();
    input.flush().unwrap();
    let mut line = String::new();
    assert!(
        output.read_line(&mut line).unwrap() > 0,
        "MCP server closed early"
    );
    serde_json::from_str(&line).unwrap()
}

fn notify(input: &mut ChildStdin, message: Value) {
    writeln!(input, "{message}").unwrap();
    input.flush().unwrap();
}

fn server(repo: &std::path::Path) -> (std::process::Child, ChildStdin, BufReader<ChildStdout>) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_excession"))
        .args(["mcp", "--repo"])
        .arg(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    let output = BufReader::new(child.stdout.take().unwrap());
    (child, input, output)
}

fn stopped(child: std::process::Child, input: ChildStdin, output: BufReader<ChildStdout>) {
    drop(input);
    drop(output);
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

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

    // An MCP client waits for initialize before sending the initialized
    // notification, then waits for tool responses by request ID. Do not rely
    // on response order for a pipelined asynchronous SDK server.
    let (child, mut input, mut output) = server(&repo);
    let initialized = rpc(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2025-11-25","capabilities":{},
                "clientInfo":{"name":"acceptance","version":"0"}}}),
    );
    assert_eq!(initialized["id"], 1);
    assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
    notify(
        &mut input,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    );
    let tools = rpc(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    );
    assert_eq!(tools["id"], 2);
    assert_eq!(
        tools["result"]["tools"].as_array().map(Vec::len),
        Some(6),
        "{tools}"
    );
    let search = rpc(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
            "name":"search_code","arguments":{"task":"renew lease","revision":"HEAD",
                "index_segments":256,"limit":4}}}),
    );
    assert_eq!(search["id"], 3);
    assert_eq!(search["result"]["isError"], false, "{search}");
    assert_eq!(
        search["result"]["structuredContent"]["source_commit"],
        commit
    );
    assert_eq!(
        search["result"]["structuredContent"]["candidates"][0]["source"]["path"],
        "src/lease.py"
    );
    let refused = rpc(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{
            "name":"get_context","arguments":{"focus":[]}}}),
    );
    assert_eq!(refused["id"], 4);
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    let unknown = rpc(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{
            "name":"shell","arguments":{}}}),
    );
    assert_eq!(unknown["id"], 5);
    assert!(unknown.get("error").is_some(), "{unknown}");
    stopped(child, input, output);
    fs::remove_dir_all(repo).unwrap();
}

/// Exercise the SDK's stateless modern lifecycle through the published binary,
/// not a local JSON-RPC dispatcher or a mock tool registry.
#[test]
fn modern_sdk_discovers_existing_tools_and_reports_refusals() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    let (child, mut input, mut output) = server(root);
    let discover = rpc(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":1,"method":"server/discover",
            "params":{"_meta":meta}}),
    );
    assert_eq!(discover["id"], 1);
    assert_eq!(discover["result"]["resultType"], "complete", "{discover}");
    let listed = rpc(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list",
            "params":{"_meta":meta}}),
    );
    assert_eq!(listed["id"], 2);
    assert_eq!(listed["result"]["resultType"], "complete", "{listed}");
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 6);
    for name in [
        "search_code",
        "get_context",
        "explain_impact",
        "verify_change",
        "plan_tests",
        "check_coverage",
    ] {
        let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
        assert_eq!(tool["annotations"]["readOnlyHint"], true);
        assert!(tool["inputSchema"]["properties"].get("repo").is_none());
    }
    let refused = rpc(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"_meta":meta,"name":"get_context","arguments":{"focus":[]}}}),
    );
    assert_eq!(refused["id"], 3);
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    let unknown = rpc(
        &mut input,
        &mut output,
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call",
            "params":{"_meta":meta,"name":"run_shell","arguments":{}}}),
    );
    assert_eq!(unknown["id"], 4);
    assert!(unknown["error"].is_object(), "{unknown}");
    stopped(child, input, output);
}
