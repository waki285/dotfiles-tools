use agent_hooks::{
    PackageManagerCheckResult, RustAllowCheckResult, check_dangerous_path_command,
    check_destructive_find, check_package_manager, check_rust_allow_attributes, has_nul_redirect,
    is_rm_command, is_rust_file,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process;

const USAGE: &str = "\
Usage:
  agent_hooks claude permission-request [flags]
  agent_hooks claude pre-tool-use [flags]
  agent_hooks copilot pre-tool-use [flags]
  agent_hooks codex pre-tool-use [flags]

Flags:
  --block-rm
  --dangerous-paths <paths>
  --deny-rust-allow
  --expect
  --additional-context <message>
  --check-package-manager
  --deny-destructive-find
  --deny-nul-redirect
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provider {
    Claude,
    Copilot,
    Codex,
}

impl Provider {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "claude" => Some(Self::Claude),
            "copilot" => Some(Self::Copilot),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    PermissionRequest,
    PreToolUse,
}

impl Event {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "permission-request" => Some(Self::PermissionRequest),
            "pre-tool-use" => Some(Self::PreToolUse),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct CliOptions {
    bash_permissions: BashPermissionOptions,
    bash_safety: BashSafetyOptions,
    rust_edits: RustEditOptions,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct BashPermissionOptions {
    block_rm: bool,
    dangerous_paths: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct BashSafetyOptions {
    check_package_manager: bool,
    deny_destructive_find: bool,
    deny_nul_redirect: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct RustEditOptions {
    deny_rust_allow: bool,
    expect: bool,
    additional_context: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedCli {
    provider: Provider,
    event: Event,
    options: CliOptions,
}

#[derive(Debug, Deserialize)]
struct ClaudeHookInput {
    tool_name: Option<String>,
    tool_input: Option<ClaudeToolInput>,
}

#[derive(Debug, Deserialize)]
struct ClaudeToolInput {
    command: Option<String>,
    new_string: Option<String>,
    content: Option<String>,
    file_path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeHookOutput {
    hook_specific_output: ClaudeHookSpecificOutput,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeHookSpecificOutput {
    hook_event_name: ClaudeHookEventName,

    #[serde(skip_serializing_if = "Option::is_none")]
    decision: Option<ClaudeDecision>,

    #[serde(skip_serializing_if = "Option::is_none")]
    permission_decision: Option<ClaudePermissionDecision>,

    #[serde(skip_serializing_if = "Option::is_none")]
    permission_decision_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
enum ClaudeHookEventName {
    PermissionRequest,
    PreToolUse,
}

#[derive(Debug, Serialize)]
struct ClaudeDecision {
    behavior: ClaudeDecisionBehavior,
    message: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum ClaudeDecisionBehavior {
    Deny,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum ClaudePermissionDecision {
    Ask,
    Deny,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CopilotHookInput {
    #[serde(default, alias = "tool_name")]
    tool_name: String,
    #[serde(default, alias = "tool_args")]
    tool_args: String,
    #[serde(default)]
    cwd: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CopilotToolArgs {
    #[serde(default)]
    command: String,
    #[serde(default, alias = "file_path", alias = "path")]
    file_path: String,
    #[serde(default, alias = "new_string")]
    new_string: String,
    #[serde(default)]
    content: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CopilotHookOutput {
    permission_decision: &'static str,
    permission_decision_reason: String,
}

#[derive(Debug, Deserialize)]
struct CodexPreToolUseInput {
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    tool_name: String,
    #[serde(default)]
    tool_input: CodexToolInput,
}

#[derive(Debug, Default, Deserialize)]
struct CodexToolInput {
    #[serde(default)]
    command: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexPreToolUseOutput {
    hook_specific_output: CodexPreToolUseHookSpecificOutput,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexPreToolUseHookSpecificOutput {
    hook_event_name: CodexHookEventName,
    permission_decision: CodexPermissionDecision,
    permission_decision_reason: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
enum CodexHookEventName {
    PreToolUse,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum CodexPermissionDecision {
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RustEdit {
    content: String,
}

#[derive(Debug, Clone, Copy)]
struct BashChecks {
    block_rm: bool,
    dangerous_paths: bool,
}

enum ParseCliResult {
    Help,
    Run(ParsedCli),
}

fn main() {
    let parsed = match parse_cli(std::env::args().skip(1)) {
        Ok(ParseCliResult::Run(parsed)) => parsed,
        Ok(ParseCliResult::Help) => {
            println!("{USAGE}");
            return;
        }
        Err(message) => {
            eprintln!("{message}\n\n{USAGE}");
            process::exit(2);
        }
    };

    let input = match read_stdin() {
        Ok(input) => input,
        Err(err) => {
            eprintln!("failed to read stdin: {err}");
            process::exit(1);
        }
    };

    match execute(&parsed, &input) {
        Ok(Some(output)) => println!("{output}"),
        Ok(None) => {}
        Err(err) => {
            eprintln!("{err}");
            process::exit(1);
        }
    }
}

fn parse_cli(args: impl Iterator<Item = String>) -> Result<ParseCliResult, String> {
    let args: Vec<String> = args.collect();
    if args.is_empty() || args.iter().any(|arg| arg == "-h" || arg == "--help") {
        return Ok(ParseCliResult::Help);
    }

    if args.len() < 2 {
        return Err("missing provider or event".to_string());
    }

    let provider =
        Provider::parse(&args[0]).ok_or_else(|| format!("unknown provider: {}", args[0]))?;
    let event = Event::parse(&args[1]).ok_or_else(|| format!("unknown event: {}", args[1]))?;

    match (provider, event) {
        (Provider::Claude, Event::PermissionRequest | Event::PreToolUse)
        | (Provider::Copilot | Provider::Codex, Event::PreToolUse) => {}
        _ => {
            return Err(format!(
                "unsupported provider/event combination: {} {}",
                args[0], args[1]
            ));
        }
    }

    let mut options = CliOptions::default();
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--block-rm" => options.bash_permissions.block_rm = true,
            "--dangerous-paths" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--dangerous-paths requires a value".to_string())?;
                options.bash_permissions.dangerous_paths = Some(value.clone());
            }
            "--deny-rust-allow" => options.rust_edits.deny_rust_allow = true,
            "--expect" => options.rust_edits.expect = true,
            "--additional-context" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--additional-context requires a value".to_string())?;
                options.rust_edits.additional_context = Some(value.clone());
            }
            "--check-package-manager" => options.bash_safety.check_package_manager = true,
            "--deny-destructive-find" => options.bash_safety.deny_destructive_find = true,
            "--deny-nul-redirect" => options.bash_safety.deny_nul_redirect = true,
            other => return Err(format!("unknown flag: {other}")),
        }
        index += 1;
    }

    validate_option_support(provider, event, &options)?;

    Ok(ParseCliResult::Run(ParsedCli {
        provider,
        event,
        options,
    }))
}

fn read_stdin() -> io::Result<String> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    Ok(input)
}

fn execute(parsed: &ParsedCli, input: &str) -> io::Result<Option<String>> {
    match (parsed.provider, parsed.event) {
        (Provider::Claude, Event::PermissionRequest) => {
            Ok(handle_claude_permission_request(&parsed.options, input))
        }
        (Provider::Claude, Event::PreToolUse) => {
            Ok(handle_claude_pre_tool_use(&parsed.options, input))
        }
        (Provider::Copilot, Event::PreToolUse) => {
            Ok(handle_copilot_pre_tool_use(&parsed.options, input))
        }
        (Provider::Codex, Event::PreToolUse) => {
            Ok(handle_codex_pre_tool_use(&parsed.options, input))
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unsupported provider/event combination",
        )),
    }
}

fn handle_claude_permission_request(options: &CliOptions, input: &str) -> Option<String> {
    if !options.bash_permissions.block_rm && options.bash_permissions.dangerous_paths.is_none() {
        return None;
    }

    let data: ClaudeHookInput = parse_json(input)?;
    if !matches_tool_name(data.tool_name.as_deref().unwrap_or_default(), &["Bash"]) {
        return None;
    }

    let cmd = data
        .tool_input
        .as_ref()
        .and_then(|tool_input| tool_input.command.as_deref())
        .unwrap_or_default();
    if cmd.is_empty() {
        return None;
    }

    if options.bash_permissions.block_rm && is_rm_command(cmd) {
        return serialize_json(&ClaudeHookOutput {
            hook_specific_output: ClaudeHookSpecificOutput {
                hook_event_name: ClaudeHookEventName::PermissionRequest,
                decision: Some(ClaudeDecision {
                    behavior: ClaudeDecisionBehavior::Deny,
                    message:
                        "rm is forbidden. Use trash command to delete files. Example: trash <path...>"
                            .to_string(),
                }),
                permission_decision: None,
                permission_decision_reason: None,
            },
        });
    }

    let paths = parse_dangerous_paths(options.bash_permissions.dangerous_paths.as_deref());
    if !paths.is_empty()
        && let Some(check) = check_dangerous_path_command(cmd, &paths)
    {
        return serialize_json(&ClaudeHookOutput {
            hook_specific_output: ClaudeHookSpecificOutput {
                hook_event_name: ClaudeHookEventName::PermissionRequest,
                decision: None,
                permission_decision: Some(ClaudePermissionDecision::Ask),
                permission_decision_reason: Some(format!(
                    "Dangerous path operation detected: {} command targeting protected path '{}'. Please confirm this operation.",
                    check.command_type, check.matched_path
                )),
            },
        });
    }

    None
}

fn handle_claude_pre_tool_use(options: &CliOptions, input: &str) -> Option<String> {
    if !options.rust_edits.deny_rust_allow
        && !options.bash_safety.check_package_manager
        && !options.bash_safety.deny_destructive_find
        && !options.bash_safety.deny_nul_redirect
    {
        return None;
    }

    let data: ClaudeHookInput = parse_json(input)?;
    let tool_name = data.tool_name.as_deref().unwrap_or_default();

    if matches_tool_name(tool_name, &["Bash"]) {
        let cmd = data
            .tool_input
            .as_ref()
            .and_then(|tool_input| tool_input.command.as_deref())
            .unwrap_or_default();

        if !cmd.is_empty()
            && let Some(reason) = evaluate_bash_denial(
                cmd,
                None,
                options,
                BashChecks {
                    block_rm: false,
                    dangerous_paths: false,
                },
            )
        {
            return serialize_json(&build_claude_pre_tool_use_denial(reason));
        }
    }

    if !options.rust_edits.deny_rust_allow || !matches_tool_name(tool_name, &["Edit", "Write"]) {
        return None;
    }

    let edit = data
        .tool_input
        .as_ref()
        .and_then(extract_claude_rust_edit)?;
    let reason = build_rust_allow_denial(options, &edit.content)?;
    serialize_json(&build_claude_pre_tool_use_denial(reason))
}

fn handle_copilot_pre_tool_use(options: &CliOptions, input: &str) -> Option<String> {
    if !options.bash_permissions.block_rm
        && options.bash_permissions.dangerous_paths.is_none()
        && !options.rust_edits.deny_rust_allow
        && !options.bash_safety.check_package_manager
        && !options.bash_safety.deny_destructive_find
        && !options.bash_safety.deny_nul_redirect
    {
        return None;
    }

    let data: CopilotHookInput = parse_json(input)?;
    if data.tool_name.trim().is_empty() {
        return None;
    }

    let tool_args = serde_json::from_str::<CopilotToolArgs>(&data.tool_args).unwrap_or_default();

    if matches_tool_name(&data.tool_name, &["bash", "shell"]) {
        let cmd = tool_args.command.trim();
        if !cmd.is_empty()
            && let Some(reason) = evaluate_bash_denial(
                cmd,
                Some(data.cwd.trim()),
                options,
                BashChecks {
                    block_rm: true,
                    dangerous_paths: true,
                },
            )
        {
            return serialize_json(&CopilotHookOutput {
                permission_decision: "deny",
                permission_decision_reason: reason,
            });
        }
    }

    if !options.rust_edits.deny_rust_allow
        || !matches_tool_name(&data.tool_name, &["edit", "write", "create"])
    {
        return None;
    }

    let edit = extract_copilot_rust_edit(&tool_args)?;
    let reason = build_rust_allow_denial(options, &edit.content)?;
    serialize_json(&CopilotHookOutput {
        permission_decision: "deny",
        permission_decision_reason: reason,
    })
}

fn handle_codex_pre_tool_use(options: &CliOptions, input: &str) -> Option<String> {
    if !options.bash_permissions.block_rm
        && options.bash_permissions.dangerous_paths.is_none()
        && !options.bash_safety.check_package_manager
        && !options.bash_safety.deny_destructive_find
        && !options.bash_safety.deny_nul_redirect
    {
        return None;
    }

    let data: CodexPreToolUseInput = parse_json(input)?;
    if !matches_tool_name(&data.tool_name, &["Bash"]) {
        return None;
    }

    let cmd = data.tool_input.command.trim();
    if cmd.is_empty() {
        return None;
    }

    let reason = evaluate_bash_denial(
        cmd,
        Some(data.cwd.trim()),
        options,
        BashChecks {
            block_rm: true,
            dangerous_paths: true,
        },
    )?;

    serialize_json(&CodexPreToolUseOutput {
        hook_specific_output: CodexPreToolUseHookSpecificOutput {
            hook_event_name: CodexHookEventName::PreToolUse,
            permission_decision: CodexPermissionDecision::Deny,
            permission_decision_reason: reason,
        },
    })
}

fn evaluate_bash_denial(
    cmd: &str,
    cwd: Option<&str>,
    options: &CliOptions,
    checks: BashChecks,
) -> Option<String> {
    if checks.block_rm && options.bash_permissions.block_rm && is_rm_command(cmd) {
        return Some(
            "rm is forbidden. Use trash command to delete files. Example: trash <path...>"
                .to_string(),
        );
    }

    if checks.dangerous_paths {
        let paths = parse_dangerous_paths(options.bash_permissions.dangerous_paths.as_deref());
        if !paths.is_empty()
            && let Some(check) = check_dangerous_path_command(cmd, &paths)
        {
            return Some(format!(
                "Dangerous path operation detected: {} command targeting protected path '{}'. Please avoid this operation.",
                check.command_type, check.matched_path
            ));
        }
    }

    if options.bash_safety.deny_nul_redirect && has_nul_redirect(cmd) {
        return Some(
            "Use /dev/null instead of nul. On Windows bash, '> nul' creates an undeletable file."
                .to_string(),
        );
    }

    if options.bash_safety.deny_destructive_find
        && let Some(description) = check_destructive_find(cmd)
    {
        return Some(format!(
            "Destructive find command detected: {description}. This operation may irreversibly delete or modify files."
        ));
    }

    if options.bash_safety.check_package_manager
        && let Some(reason) = build_package_manager_mismatch(cmd, cwd)
    {
        return Some(reason);
    }

    None
}

fn build_package_manager_mismatch(cmd: &str, cwd: Option<&str>) -> Option<String> {
    let start_dir = parse_start_dir(cwd.unwrap_or_default());
    match check_package_manager(cmd, Path::new(&start_dir)) {
        PackageManagerCheckResult::Mismatch {
            command_pm,
            expected_pm,
        } => Some(format!(
            "Package manager mismatch: This project uses {} (detected {}), but you are trying to use {}. Please use {} instead.",
            expected_pm.name(),
            expected_pm.lock_files()[0],
            command_pm.name(),
            expected_pm.name()
        )),
        _ => None,
    }
}

fn build_rust_allow_denial(options: &CliOptions, content: &str) -> Option<String> {
    let check_result = check_rust_allow_attributes(content);
    let base_message = if options.rust_edits.expect {
        match check_result {
            RustAllowCheckResult::HasAllow | RustAllowCheckResult::HasBoth => Some(
                "Adding #[allow(...)] or #![allow(...)] attributes is not permitted. Use #[expect(...)] instead, which will warn when the lint is no longer triggered.",
            ),
            _ => None,
        }
    } else {
        match check_result {
            RustAllowCheckResult::Ok => None,
            RustAllowCheckResult::HasBoth => Some(
                "Adding #[allow(...)] or #[expect(...)] attributes is not permitted. Fix the underlying issue instead of suppressing the warning.",
            ),
            RustAllowCheckResult::HasAllow => Some(
                "Adding #[allow(...)] or #![allow(...)] attributes is not permitted. Fix the underlying issue instead of suppressing the warning.",
            ),
            RustAllowCheckResult::HasExpect => Some(
                "Adding #[expect(...)] or #![expect(...)] attributes is not permitted. Fix the underlying issue instead of suppressing the warning.",
            ),
        }
    }?;

    let mut result = base_message.to_string();
    if let Some(extra_context) = options.rust_edits.additional_context.as_deref() {
        result.push(' ');
        result.push_str(extra_context);
    }
    Some(result)
}

const fn build_claude_pre_tool_use_denial(reason: String) -> ClaudeHookOutput {
    ClaudeHookOutput {
        hook_specific_output: ClaudeHookSpecificOutput {
            hook_event_name: ClaudeHookEventName::PreToolUse,
            decision: None,
            permission_decision: Some(ClaudePermissionDecision::Deny),
            permission_decision_reason: Some(reason),
        },
    }
}

fn validate_option_support(
    provider: Provider,
    event: Event,
    options: &CliOptions,
) -> Result<(), String> {
    let mut unsupported = Vec::new();

    let supports_block_rm = matches!(
        (provider, event),
        (Provider::Claude, Event::PermissionRequest)
            | (Provider::Copilot | Provider::Codex, Event::PreToolUse)
    );
    let supports_dangerous_paths = supports_block_rm;
    let supports_rust_allow = matches!(
        (provider, event),
        (Provider::Claude | Provider::Copilot, Event::PreToolUse)
    );
    let supports_expect = supports_rust_allow;
    let supports_additional_context = supports_rust_allow;
    let supports_pm_checks = matches!(
        (provider, event),
        (
            Provider::Claude | Provider::Copilot | Provider::Codex,
            Event::PreToolUse
        )
    );
    let supports_destructive_find = supports_pm_checks;
    let supports_nul_redirect = supports_pm_checks;

    if options.bash_permissions.block_rm && !supports_block_rm {
        unsupported.push("--block-rm");
    }
    if options.bash_permissions.dangerous_paths.is_some() && !supports_dangerous_paths {
        unsupported.push("--dangerous-paths");
    }
    if options.rust_edits.deny_rust_allow && !supports_rust_allow {
        unsupported.push("--deny-rust-allow");
    }
    if options.rust_edits.expect && !supports_expect {
        unsupported.push("--expect");
    }
    if options.rust_edits.additional_context.is_some() && !supports_additional_context {
        unsupported.push("--additional-context");
    }
    if options.bash_safety.check_package_manager && !supports_pm_checks {
        unsupported.push("--check-package-manager");
    }
    if options.bash_safety.deny_destructive_find && !supports_destructive_find {
        unsupported.push("--deny-destructive-find");
    }
    if options.bash_safety.deny_nul_redirect && !supports_nul_redirect {
        unsupported.push("--deny-nul-redirect");
    }

    if unsupported.is_empty() {
        return Ok(());
    }

    Err(format!(
        "unsupported flag(s) for this command: {}",
        unsupported.join(", ")
    ))
}

fn extract_claude_rust_edit(tool_input: &ClaudeToolInput) -> Option<RustEdit> {
    let file_path = tool_input.file_path.as_deref().unwrap_or_default();
    if file_path.is_empty() || !is_rust_file(file_path) {
        return None;
    }

    let content = tool_input
        .new_string
        .as_deref()
        .or(tool_input.content.as_deref())
        .unwrap_or_default();
    if content.is_empty() {
        return None;
    }

    Some(RustEdit {
        content: content.to_string(),
    })
}

fn extract_copilot_rust_edit(tool_args: &CopilotToolArgs) -> Option<RustEdit> {
    let file_path = tool_args.file_path.trim();
    if file_path.is_empty() || !is_rust_file(file_path) {
        return None;
    }

    let content = if tool_args.new_string.is_empty() {
        tool_args.content.as_str()
    } else {
        tool_args.new_string.as_str()
    };
    if content.is_empty() {
        return None;
    }

    Some(RustEdit {
        content: content.to_string(),
    })
}

fn parse_dangerous_paths(paths: Option<&str>) -> Vec<&str> {
    paths
        .into_iter()
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect()
}

fn parse_start_dir(cwd: &str) -> PathBuf {
    if !cwd.is_empty() {
        return PathBuf::from(cwd);
    }

    std::env::current_dir().unwrap_or_default()
}

fn matches_tool_name(tool_name: &str, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| tool_name.eq_ignore_ascii_case(candidate))
}

fn parse_json<T: DeserializeOwned>(input: &str) -> Option<T> {
    serde_json::from_str(input).ok()
}

fn serialize_json<T: Serialize>(value: &T) -> Option<String> {
    serde_json::to_string(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn run_hook(parsed: &ParsedCli, input: &str) -> Option<Value> {
        execute(parsed, input)
            .unwrap()
            .map(|output| serde_json::from_str(&output).unwrap())
    }

    #[test]
    fn parse_cli_rejects_codex_rust_flags() {
        let result = parse_cli(
            ["codex", "pre-tool-use", "--deny-rust-allow"]
                .into_iter()
                .map(String::from),
        );

        assert!(result.is_err());
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
}
