//! Recipe-root discovery over stdio: without COOK_RECIPES_DIR the server asks
//! the client for its MCP roots, and ignores `/` and plugin install folders.

use std::io::{BufRead, BufReader, Lines, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

struct Client {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    /// Answer to the server's `roots/list` requests; `None` = never asked.
    roots: Vec<String>,
    roots_requests: usize,
}

impl Client {
    fn spawn(cwd: &Path, capabilities: Value) -> Self {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_cook-mcp"));
        cmd.current_dir(cwd)
            .env_remove("COOK_RECIPES_DIR")
            .env("COOK_MCP_AUTH_PATH", "/nonexistent/auth.json")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        for k in ["CLAUDE_PLUGIN_ROOT", "CURSOR_PLUGIN_ROOT", "PLUGIN_ROOT"] {
            cmd.env_remove(k);
        }
        let mut child = cmd.spawn().expect("spawn server");
        let stdin = child.stdin.take().unwrap();
        let lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut c = Self {
            child,
            stdin,
            lines,
            roots: vec![],
            roots_requests: 0,
        };
        c.send(json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18",
                        "capabilities": capabilities,
                        "clientInfo": {"name": "roots-test", "version": "0"}}
        }));
        assert_eq!(c.response(1)["result"]["serverInfo"]["name"], "cook-mcp");
        c.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        c
    }

    fn send(&mut self, msg: Value) {
        writeln!(self.stdin, "{}", serde_json::to_string(&msg).unwrap()).unwrap();
    }

    /// Read until the response to `id`, answering `roots/list` on the way.
    fn response(&mut self, id: u64) -> Value {
        loop {
            let msg: Value =
                serde_json::from_str(&self.lines.next().expect("server closed").unwrap()).unwrap();
            if msg["method"] == "roots/list" {
                self.roots_requests += 1;
                let roots: Vec<Value> = self.roots.iter().map(|u| json!({"uri": u})).collect();
                let id = msg["id"].clone();
                self.send(json!({"jsonrpc": "2.0", "id": id, "result": {"roots": roots}}));
            } else if msg["id"] == id {
                return msg;
            }
        }
    }

    fn call(&mut self, id: u64, tool: &str) -> Value {
        self.send(json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": tool, "arguments": {}}
        }));
        self.response(id)["result"].clone()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn text(result: &Value) -> String {
    result["content"][0]["text"].as_str().unwrap().to_string()
}

fn uri(p: &Path) -> String {
    format!("file://{}", p.display()).replace(' ', "%20")
}

fn collection(dir: &Path, recipe: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(recipe), "Boil @water{1%l}.\n").unwrap();
}

#[cfg(unix)]
#[test]
fn client_roots_choose_the_recipe_folder_when_started_in_root_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let first = tmp.path().join("My Recipes");
    let second = tmp.path().join("other");
    collection(&first, "Soup.cook");
    collection(&second, "Stew.cook");

    let mut c = Client::spawn(Path::new("/"), json!({"roots": {"listChanged": true}}));
    c.roots = vec![
        "https://example.com/not-a-folder".into(),
        uri(&tmp.path().join("missing")),
        uri(&first),
    ];
    let r = c.call(2, "list_recipes");
    assert_ne!(r["isError"], true, "{r}");
    assert!(text(&r).contains("Soup.cook"), "{r}");
    assert_eq!(c.roots_requests, 1);

    // Cached until the client says the roots changed.
    c.call(3, "list_recipes");
    assert_eq!(c.roots_requests, 1);

    let status: Value = serde_json::from_str(&text(&c.call(4, "auth_status"))).unwrap();
    assert_eq!(status["recipe_root_source"], "roots");
    assert!(
        status["recipe_root"]
            .as_str()
            .unwrap()
            .ends_with("My Recipes"),
        "{status}"
    );

    c.roots = vec![uri(&second)];
    c.send(json!({"jsonrpc": "2.0", "method": "notifications/roots/list_changed"}));
    let r = c.call(5, "list_recipes");
    assert!(text(&r).contains("Stew.cook"), "{r}");
    assert_eq!(c.roots_requests, 2);

    // Roots that give nothing usable fall back to the (unset) working dir.
    c.roots = vec![];
    c.send(json!({"jsonrpc": "2.0", "method": "notifications/roots/list_changed"}));
    let r = c.call(6, "list_recipes");
    assert_eq!(r["isError"], true, "{r}");
    assert!(text(&r).contains("No recipe folder set"), "{r}");
}

#[cfg(unix)]
#[test]
fn without_roots_capability_root_dir_is_unset_and_roots_are_not_requested() {
    let mut c = Client::spawn(Path::new("/"), json!({}));
    let r = c.call(2, "list_recipes");
    assert_eq!(r["isError"], true, "{r}");
    assert!(text(&r).contains("COOK_RECIPES_DIR"), "{r}");
    assert_eq!(c.roots_requests, 0);
    let status: Value = serde_json::from_str(&text(&c.call(3, "auth_status"))).unwrap();
    assert_eq!(status["recipe_root_source"], "unset");
}

#[test]
fn plugin_install_folder_is_not_a_recipe_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let plugin = tmp
        .path()
        .join(".codex/plugins/cache/cooklang-skills/cooklang/2.0.0");
    collection(&plugin.join("examples"), "Example.cook");

    let mut c = Client::spawn(&plugin, json!({}));
    let r = c.call(2, "list_recipes");
    assert_eq!(r["isError"], true, "{r}");
    assert!(text(&r).contains("No recipe folder set"), "{r}");

    // A project folder still works as before.
    let proj = tmp.path().join("proj");
    collection(&proj, "Mine.cook");
    let mut c = Client::spawn(&proj, json!({}));
    let r = c.call(2, "list_recipes");
    assert!(text(&r).contains("Mine.cook"), "{r}");
    let status: Value = serde_json::from_str(&text(&c.call(3, "auth_status"))).unwrap();
    assert_eq!(status["recipe_root_source"], "cwd");
}
