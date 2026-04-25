use agent_hooks::{
    PackageManagerCheckResult, RustAllowCheckResult, check_dangerous_path_command,
    check_destructive_find, check_package_manager, check_rust_allow_attributes, has_nul_redirect,
    is_rm_command, is_rust_file,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::CliOptions;

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
struct CodexHookInput {
    #[serde(default)]
    cwd: String,
    #[serde(default)]
    tool_name: String,
    #[serde(default)]
    tool_input: Value,
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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexPermissionRequestOutput {
    hook_specific_output: CodexPermissionRequestHookSpecificOutput,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexPermissionRequestHookSpecificOutput {
    hook_event_name: CodexHookEventName,
    decision: CodexPermissionRequestDecision,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CodexPermissionRequestDecision {
    behavior: CodexPermissionDecision,
    message: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
enum CodexHookEventName {
    PermissionRequest,
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

pub fn handle_claude_permission_request(options: &CliOptions, input: &str) -> Option<String> {
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

pub fn handle_claude_pre_tool_use(options: &CliOptions, input: &str) -> Option<String> {
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

pub fn handle_copilot_pre_tool_use(options: &CliOptions, input: &str) -> Option<String> {
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

pub fn handle_codex_pre_tool_use(options: &CliOptions, input: &str) -> Option<String> {
    if !options.bash_permissions.block_rm
        && options.bash_permissions.dangerous_paths.is_none()
        && !options.rust_edits.deny_rust_allow
        && !options.bash_safety.check_package_manager
        && !options.bash_safety.deny_destructive_find
        && !options.bash_safety.deny_nul_redirect
    {
        return None;
    }

    let data: CodexHookInput = parse_json(input)?;
    let tool_name = data.tool_name.trim();

    if matches_tool_name(tool_name, &["Bash"])
        && let Some(cmd) = extract_codex_command(&data.tool_input)
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
        return serialize_json(&CodexPreToolUseOutput {
            hook_specific_output: CodexPreToolUseHookSpecificOutput {
                hook_event_name: CodexHookEventName::PreToolUse,
                permission_decision: CodexPermissionDecision::Deny,
                permission_decision_reason: reason,
            },
        });
    }

    if !options.rust_edits.deny_rust_allow {
        return None;
    }

    let edit = extract_codex_rust_edit(tool_name, &data.tool_input)?;
    let reason = build_rust_allow_denial(options, &edit.content)?;

    serialize_json(&CodexPreToolUseOutput {
        hook_specific_output: CodexPreToolUseHookSpecificOutput {
            hook_event_name: CodexHookEventName::PreToolUse,
            permission_decision: CodexPermissionDecision::Deny,
            permission_decision_reason: reason,
        },
    })
}

pub fn handle_codex_permission_request(options: &CliOptions, input: &str) -> Option<String> {
    if !options.bash_permissions.block_rm && options.bash_permissions.dangerous_paths.is_none() {
        return None;
    }

    let data: CodexHookInput = parse_json(input)?;
    if !matches_tool_name(&data.tool_name, &["Bash"]) {
        return None;
    }

    let cmd = extract_codex_command(&data.tool_input)?;
    let reason = evaluate_bash_denial(
        cmd,
        Some(data.cwd.trim()),
        options,
        BashChecks {
            block_rm: true,
            dangerous_paths: true,
        },
    )?;

    serialize_json(&CodexPermissionRequestOutput {
        hook_specific_output: CodexPermissionRequestHookSpecificOutput {
            hook_event_name: CodexHookEventName::PermissionRequest,
            decision: CodexPermissionRequestDecision {
                behavior: CodexPermissionDecision::Deny,
                message: reason,
            },
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

fn extract_codex_command(tool_input: &Value) -> Option<&str> {
    tool_input
        .get("command")?
        .as_str()
        .map(str::trim)
        .filter(|command| !command.is_empty())
}

fn extract_codex_rust_edit(tool_name: &str, tool_input: &Value) -> Option<RustEdit> {
    if !matches_tool_name(tool_name, &["apply_patch", "Edit", "Write"]) {
        return None;
    }

    let content = extract_apply_patch_rust_additions(extract_codex_command(tool_input)?)?;
    Some(RustEdit { content })
}

fn extract_apply_patch_rust_additions(patch: &str) -> Option<String> {
    let mut current_is_rust = false;
    let mut additions = Vec::new();

    for line in patch.lines() {
        if let Some(path) = line
            .strip_prefix("*** Add File: ")
            .or_else(|| line.strip_prefix("*** Update File: "))
            .or_else(|| line.strip_prefix("*** Move to: "))
        {
            current_is_rust = is_rust_file(path.trim());
            continue;
        }

        if line.starts_with("*** Delete File: ") {
            current_is_rust = false;
            continue;
        }

        if current_is_rust && let Some(added_line) = line.strip_prefix('+') {
            additions.push(added_line.to_string());
        }
    }

    if additions.is_empty() {
        None
    } else {
        Some(additions.join("\n"))
    }
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
