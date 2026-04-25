use serde_json::Value;

use super::*;

fn run_hook(parsed: &ParsedCli, input: &str) -> Option<Value> {
    execute(parsed, input)
        .unwrap()
        .map(|output| serde_json::from_str(&output).unwrap())
}

#[test]
fn parse_cli_accepts_codex_rust_flags() {
    let result = parse_cli(
        ["codex", "pre-tool-use", "--deny-rust-allow"]
            .into_iter()
            .map(String::from),
    );

    assert!(matches!(result, Ok(ParseCliResult::Run(_))));
}

#[test]
fn parse_cli_rejects_claude_permission_request_rust_flags() {
    let result = parse_cli(
        ["claude", "permission-request", "--deny-rust-allow"]
            .into_iter()
            .map(String::from),
    );

    assert!(result.is_err());
}

#[test]
fn parse_cli_accepts_codex_permission_request() {
    let result = parse_cli(
        ["codex", "permission-request", "--block-rm"]
            .into_iter()
            .map(String::from),
    );

    assert!(matches!(result, Ok(ParseCliResult::Run(_))));
}

#[test]
fn claude_permission_request_blocks_rm() {
    let parsed = ParsedCli {
        provider: Provider::Claude,
        event: Event::PermissionRequest,
        options: CliOptions {
            bash_permissions: BashPermissionOptions {
                block_rm: true,
                ..BashPermissionOptions::default()
            },
            ..CliOptions::default()
        },
    };

    let output = run_hook(
        &parsed,
        r#"{"tool_name":"Bash","tool_input":{"command":"rm -rf /tmp/test"}}"#,
    )
    .unwrap();

    assert_eq!(
        output["hookSpecificOutput"]["decision"]["behavior"],
        Value::String("deny".to_string())
    );
}

#[test]
fn claude_pre_tool_use_denies_rust_allow() {
    let parsed = ParsedCli {
        provider: Provider::Claude,
        event: Event::PreToolUse,
        options: CliOptions {
            rust_edits: RustEditOptions {
                deny_rust_allow: true,
                expect: true,
                ..RustEditOptions::default()
            },
            ..CliOptions::default()
        },
    };

    let output = run_hook(
        &parsed,
        r##"{"tool_name":"Edit","tool_input":{"file_path":"src/main.rs","new_string":"#[allow(dead_code)]"}}"##,
    )
    .unwrap();

    assert_eq!(
        output["hookSpecificOutput"]["permissionDecision"],
        Value::String("deny".to_string())
    );
}

#[test]
fn copilot_pre_tool_use_blocks_rm() {
    let parsed = ParsedCli {
        provider: Provider::Copilot,
        event: Event::PreToolUse,
        options: CliOptions {
            bash_permissions: BashPermissionOptions {
                block_rm: true,
                ..BashPermissionOptions::default()
            },
            ..CliOptions::default()
        },
    };

    let output = run_hook(
        &parsed,
        r#"{"toolName":"bash","toolArgs":"{\"command\":\"rm -rf /tmp/test\"}","cwd":"/repo"}"#,
    )
    .unwrap();

    assert_eq!(
        output["permissionDecision"],
        Value::String("deny".to_string())
    );
}

#[test]
fn codex_pre_tool_use_denies_rm() {
    let parsed = ParsedCli {
        provider: Provider::Codex,
        event: Event::PreToolUse,
        options: CliOptions {
            bash_permissions: BashPermissionOptions {
                block_rm: true,
                ..BashPermissionOptions::default()
            },
            ..CliOptions::default()
        },
    };

    let output = run_hook(
        &parsed,
        r#"{"session_id":"session","transcript_path":null,"cwd":"/repo","hook_event_name":"PreToolUse","model":"gpt-5.4","permission_mode":"default","turn_id":"turn","tool_name":"Bash","tool_use_id":"tool","tool_input":{"command":"rm -rf /tmp/test"}}"#,
    )
    .unwrap();

    assert_eq!(
        output["hookSpecificOutput"]["hookEventName"],
        Value::String("PreToolUse".to_string())
    );
    assert_eq!(
        output["hookSpecificOutput"]["permissionDecision"],
        Value::String("deny".to_string())
    );
}

#[test]
fn codex_pre_tool_use_ignores_non_bash() {
    let parsed = ParsedCli {
        provider: Provider::Codex,
        event: Event::PreToolUse,
        options: CliOptions {
            bash_permissions: BashPermissionOptions {
                block_rm: true,
                ..BashPermissionOptions::default()
            },
            ..CliOptions::default()
        },
    };

    let output = run_hook(
        &parsed,
        r#"{"session_id":"session","transcript_path":null,"cwd":"/repo","hook_event_name":"PreToolUse","model":"gpt-5.4","permission_mode":"default","turn_id":"turn","tool_name":"Write","tool_use_id":"tool","tool_input":{"command":"rm -rf /tmp/test"}}"#,
    );

    assert!(output.is_none());
}

#[test]
fn codex_pre_tool_use_denies_rust_allow_in_apply_patch() {
    let parsed = ParsedCli {
        provider: Provider::Codex,
        event: Event::PreToolUse,
        options: CliOptions {
            rust_edits: RustEditOptions {
                deny_rust_allow: true,
                expect: true,
                ..RustEditOptions::default()
            },
            ..CliOptions::default()
        },
    };

    let output = run_hook(
        &parsed,
        r#"{"session_id":"session","transcript_path":null,"cwd":"/repo","hook_event_name":"PreToolUse","model":"gpt-5.4","permission_mode":"default","turn_id":"turn","tool_name":"apply_patch","tool_use_id":"tool","tool_input":{"command":"*** Begin Patch\n*** Update File: src/main.rs\n@@\n+#[allow(dead_code)]\n*** End Patch\n"}}"#,
    )
    .unwrap();

    assert_eq!(
        output["hookSpecificOutput"]["permissionDecision"],
        Value::String("deny".to_string())
    );
}

#[test]
fn codex_pre_tool_use_denies_package_manager_mismatch() {
    let temp_dir = std::env::temp_dir().join("agent_hooks_cli_codex_pm");
    let _ = std::fs::create_dir_all(&temp_dir);
    std::fs::write(temp_dir.join("pnpm-lock.yaml"), "").unwrap();

    let parsed = ParsedCli {
        provider: Provider::Codex,
        event: Event::PreToolUse,
        options: CliOptions {
            bash_safety: BashSafetyOptions {
                check_package_manager: true,
                ..BashSafetyOptions::default()
            },
            ..CliOptions::default()
        },
    };
    let escaped_cwd = temp_dir.display().to_string().replace('\\', "\\\\");

    let output = run_hook(
        &parsed,
        &format!(
            r#"{{"session_id":"session","transcript_path":null,"cwd":"{escaped_cwd}","hook_event_name":"PreToolUse","model":"gpt-5.4","permission_mode":"default","turn_id":"turn","tool_name":"Bash","tool_use_id":"tool","tool_input":{{"command":"npm install"}}}}"#
        ),
    )
    .unwrap();

    assert_eq!(
        output["hookSpecificOutput"]["permissionDecision"],
        Value::String("deny".to_string())
    );

    let _ = std::fs::remove_file(temp_dir.join("pnpm-lock.yaml"));
    let _ = std::fs::remove_dir(&temp_dir);
}

#[test]
fn codex_permission_request_blocks_rm() {
    let parsed = ParsedCli {
        provider: Provider::Codex,
        event: Event::PermissionRequest,
        options: CliOptions {
            bash_permissions: BashPermissionOptions {
                block_rm: true,
                ..BashPermissionOptions::default()
            },
            ..CliOptions::default()
        },
    };

    let output = run_hook(
        &parsed,
        r#"{"session_id":"session","transcript_path":null,"cwd":"/repo","hook_event_name":"PermissionRequest","model":"gpt-5.4","permission_mode":"default","turn_id":"turn","tool_name":"Bash","tool_input":{"command":"rm -rf /tmp/test"}}"#,
    )
    .unwrap();

    assert_eq!(
        output["hookSpecificOutput"]["hookEventName"],
        Value::String("PermissionRequest".to_string())
    );
    assert_eq!(
        output["hookSpecificOutput"]["decision"]["behavior"],
        Value::String("deny".to_string())
    );
}
