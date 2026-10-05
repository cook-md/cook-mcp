use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn rpc(line: &serde_json::Value) -> String {
    serde_json::to_string(line).unwrap()
}

#[test]
fn initialize_and_list_tools_over_stdio() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cook-mcp"))
        .env(
            "COOK_RECIPES_DIR",
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/workspace"),
        )
        .env("COOK_MCP_AUTH_PATH", "/nonexistent/auth.json")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();

    writeln!(
        stdin,
        "{}",
        rpc(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18",
                        "capabilities": {},
                        "clientInfo": {"name": "smoke", "version": "0"}}
        }))
    )
    .unwrap();
    let resp: serde_json::Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["serverInfo"]["name"], "cook-mcp");

    writeln!(
        stdin,
        "{}",
        rpc(&serde_json::json!({
            "jsonrpc": "2.0", "method": "notifications/initialized"
        }))
    )
    .unwrap();
    writeln!(
        stdin,
        "{}",
        rpc(&serde_json::json!({
            "jsonrpc": "2.0", "id": 2, "method": "tools/list"
        }))
    )
    .unwrap();
    let resp: serde_json::Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let names: Vec<&str> = resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for expected in [
        "list_recipes",
        "read_recipe",
        "search_recipes",
        "validate",
        "write_recipe",
        "write_menu",
        "write_config",
        "shopping_list",
        "pantry_list",
        "pantry_expiring",
        "pantry_depleted",
        "pantry_recipes",
        "pantry_update",
        "get_nutrition",
        "aggregate_nutrition",
        "lookup_ingredient",
        "convert_units",
        "check_category",
        "branded_lookup",
        "reference_intakes",
        "render_report",
        "import_recipe",
        "login",
        "auth_status",
    ] {
        assert!(
            names.contains(&expected),
            "missing {expected}; tools were: {names:?}"
        );
    }

    writeln!(
        stdin,
        "{}",
        rpc(&serde_json::json!({
            "jsonrpc": "2.0", "id": 3, "method": "prompts/list"
        }))
    )
    .unwrap();
    let resp: serde_json::Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let prompts: Vec<&str> = resp["result"]["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    for expected in [
        "meal-planning",
        "shopping-list",
        "pantry",
        "import-recipe",
        "edit-recipe",
        "nutrition-report",
        "nutrition-goals",
    ] {
        assert!(prompts.contains(&expected), "prompts were: {prompts:?}");
    }

    writeln!(
        stdin,
        "{}",
        rpc(&serde_json::json!({"jsonrpc": "2.0", "id": 4, "method": "resources/list"}))
    )
    .unwrap();
    let resp: serde_json::Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let uris: Vec<&str> = resp["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    for expected in [
        "cooklang://spec",
        "cooklang://syntax",
        "cooklang://menu-format",
        "cooklang://skills/meal-planning",
    ] {
        assert!(uris.contains(&expected), "resources were: {uris:?}");
    }

    writeln!(
        stdin,
        "{}",
        rpc(&serde_json::json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": {"name": "validate", "arguments": {}}
        }))
    )
    .unwrap();
    let resp: serde_json::Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        text.contains("\"total_recipes\": 4"),
        "validate over fixture: {text}"
    );

    drop(stdin);
    let _ = child.wait();
}

#[test]
fn unknown_subcommand_exits_2_with_usage() {
    let out = Command::new(env!("CARGO_BIN_EXE_cook-mcp"))
        .arg("badcmd")
        .stdin(Stdio::null())
        .output()
        .expect("run binary");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("usage: cook-mcp"), "stderr was: {stderr}");
}
