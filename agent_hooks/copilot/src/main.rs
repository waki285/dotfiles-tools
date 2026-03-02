use agent_hooks::{
    PackageManagerCheckResult, RustAllowCheckResult, check_dangerous_path_command,
    check_destructive_find, check_package_manager, check_rust_allow_attributes, has_nul_redirect,
    is_rm_command, is_rust_file,
};
use seahorse::{App, Command, Context, Flag, FlagType};
use serde::{Deserialize, Serialize};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HookInput {
    #[serde(default, alias = "tool_name")]
    tool_name: String,
    #[serde(default, alias = "tool_args")]
    tool_args: String,
    #[serde(default)]
    cwd: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolArgs {
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
struct HookOutput {
    permission_decision: &'static str,
    permission_decision_reason: String,
}

fn read_hook_input() -> io::Result<HookInput> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    serde_json::from_str(&input).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn output_deny(reason: impl Into<String>) {
    let output = HookOutput {
        permission_decision: "deny",
        permission_decision_reason: reason.into(),
    };

    if let Ok(json) = serde_json::to_string(&output) {
        println!("{json}");
    }
}

fn is_tool_name(tool_name: &str, candidates: &[&str]) -> bool {
    candidates
        .iter()
        .any(|candidate| tool_name.eq_ignore_ascii_case(candidate))
}

fn parse_start_dir(cwd: &str) -> PathBuf {
    if !cwd.is_empty() {
        return PathBuf::from(cwd);
    }

    std::env::current_dir().unwrap_or_default()
}

fn handle_package_manager_check(cmd: &str, cwd: &str) -> Option<String> {
    let start_dir = parse_start_dir(cwd);
    match check_package_manager(cmd, Path::new(&start_dir)) {
        PackageManagerCheckResult::Mismatch {
            command_pm,
            expected_pm,
        } => Some(format!(
            "Package manager mismatch: This project uses {} (detected {}), \
             but you are trying to use {}. Please use {} instead.",
            expected_pm.name(),
            expected_pm.lock_files()[0],
            command_pm.name(),
            expected_pm.name()
        )),
        _ => None,
    }
}

fn build_rust_allow_denial_reason(
    check_result: RustAllowCheckResult,
    expect_flag: bool,
    additional_context: Option<&str>,
) -> Option<String> {
    let base_msg = if expect_flag {
        match check_result {
            RustAllowCheckResult::HasAllow | RustAllowCheckResult::HasBoth => Some(
                "Adding #[allow(...)] or #![allow(...)] attributes is not permitted. \
                 Use #[expect(...)] instead, which will warn when the lint is no longer triggered.",
            ),
            _ => None,
        }
    } else {
        match check_result {
            RustAllowCheckResult::Ok => None,
            RustAllowCheckResult::HasBoth => Some(
                "Adding #[allow(...)] or #[expect(...)] attributes is not permitted. \
                 Fix the underlying issue instead of suppressing the warning.",
            ),
            RustAllowCheckResult::HasAllow => Some(
                "Adding #[allow(...)] or #![allow(...)] attributes is not permitted. \
                 Fix the underlying issue instead of suppressing the warning.",
            ),
            RustAllowCheckResult::HasExpect => Some(
                "Adding #[expect(...)] or #![expect(...)] attributes is not permitted. \
                 Fix the underlying issue instead of suppressing the warning.",
            ),
        }
    };

    base_msg.map(|msg| {
        let mut result = msg.to_string();
        if let Some(ctx) = additional_context {
            result.push(' ');
            result.push_str(ctx);
        }
        result
    })
}

fn pre_tool_use_action(c: &Context) {
    let block_rm_enabled = c.bool_flag("block-rm");
    let dangerous_paths = c.string_flag("dangerous-paths").ok();
    let deny_rust_allow_enabled = c.bool_flag("deny-rust-allow");
    let check_package_manager_enabled = c.bool_flag("check-package-manager");
    let deny_destructive_find_enabled = c.bool_flag("deny-destructive-find");
    let deny_nul_redirect_enabled = c.bool_flag("deny-nul-redirect");

    if !block_rm_enabled
        && dangerous_paths.is_none()
        && !deny_rust_allow_enabled
        && !check_package_manager_enabled
        && !deny_destructive_find_enabled
        && !deny_nul_redirect_enabled
    {
        return;
    }

    let Ok(input) = read_hook_input() else {
        return;
    };

    let tool_name = input.tool_name.trim();
    if tool_name.is_empty() {
        return;
    }

    let tool_args = serde_json::from_str::<ToolArgs>(&input.tool_args).unwrap_or_default();

    if is_tool_name(tool_name, &["bash", "shell"]) {
        let cmd = tool_args.command.trim();
        if !cmd.is_empty() {
            if block_rm_enabled && is_rm_command(cmd) {
                output_deny(
                    "rm is forbidden. Use trash command to delete files. Example: trash <path...>",
                );
                return;
            }

            if let Some(ref paths_str) = dangerous_paths {
                let paths: Vec<&str> = paths_str
                    .split(',')
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .collect();
                if let Some(check) = check_dangerous_path_command(cmd, &paths) {
                    output_deny(format!(
                        "Dangerous path operation detected: {} command targeting protected path '{}'. \
                         Please avoid this operation.",
                        check.command_type, check.matched_path
                    ));
                    return;
                }
            }

            if deny_nul_redirect_enabled && has_nul_redirect(cmd) {
                output_deny(
                    "Use /dev/null instead of nul. On Windows bash, '> nul' creates an undeletable file.",
                );
                return;
            }

            if deny_destructive_find_enabled && let Some(description) = check_destructive_find(cmd)
            {
                output_deny(format!(
                    "Destructive find command detected: {description}. \
                     This operation may irreversibly delete or modify files."
                ));
                return;
            }

            if check_package_manager_enabled
                && let Some(reason) = handle_package_manager_check(cmd, input.cwd.trim())
            {
                output_deny(reason);
                return;
            }
        }
    }

    if !deny_rust_allow_enabled || !is_tool_name(tool_name, &["edit", "write", "create"]) {
        return;
    }

    let file_path = tool_args.file_path.trim();
    if file_path.is_empty() || !is_rust_file(file_path) {
        return;
    }

    let content = if tool_args.new_string.is_empty() {
        tool_args.content.as_str()
    } else {
        tool_args.new_string.as_str()
    };

    if content.is_empty() {
        return;
    }

    let expect_flag = c.bool_flag("expect");
    let additional_context = c.string_flag("additional-context").ok();
    let check_result = check_rust_allow_attributes(content);

    if let Some(reason) =
        build_rust_allow_denial_reason(check_result, expect_flag, additional_context.as_deref())
    {
        output_deny(reason);
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let app = App::new(env!("CARGO_PKG_NAME"))
        .description(env!("CARGO_PKG_DESCRIPTION"))
        .version(env!("CARGO_PKG_VERSION"))
        .command(
            Command::new("pre-tool-use")
                .description("Handle pre-tool-use checks for Copilot CLI hooks")
                .flag(
                    Flag::new("block-rm", FlagType::Bool)
                        .description("Block rm command and suggest using trash instead"),
                )
                .flag(
                    Flag::new("dangerous-paths", FlagType::String)
                        .description("Comma-separated list of dangerous paths to protect from rm/trash/mv"),
                )
                .flag(
                    Flag::new("deny-rust-allow", FlagType::Bool)
                        .description("Deny #[allow(...)] attributes in Rust files"),
                )
                .flag(
                    Flag::new("expect", FlagType::Bool)
                        .description("With --deny-rust-allow: suggest #[expect(...)] instead of denying both"),
                )
                .flag(
                    Flag::new("additional-context", FlagType::String).description(
                        "With --deny-rust-allow: additional context message to append to the denial reason",
                    ),
                )
                .flag(
                    Flag::new("check-package-manager", FlagType::Bool)
                        .description("Check for package manager mismatch (e.g., using npm when pnpm-lock.yaml exists)"),
                )
                .flag(
                    Flag::new("deny-destructive-find", FlagType::Bool)
                        .description("Deny destructive find commands (e.g., find -delete, find -exec rm)"),
                )
                .flag(
                    Flag::new("deny-nul-redirect", FlagType::Bool)
                        .description("Deny redirects to nul on Windows (e.g., > nul, 2> nul, &> nul)"),
                )
                .action(pre_tool_use_action),
        );

    app.run(args);
}
