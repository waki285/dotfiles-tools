mod hooks;
#[cfg(test)]
mod tests;

use std::io::{self, Read};
use std::process;

use hooks::{
    handle_claude_permission_request, handle_claude_pre_tool_use, handle_codex_permission_request,
    handle_codex_pre_tool_use, handle_copilot_pre_tool_use,
};

const USAGE: &str = "\
Usage:
  agent_hooks claude permission-request [flags]
  agent_hooks claude pre-tool-use [flags]
  agent_hooks copilot pre-tool-use [flags]
  agent_hooks codex permission-request [flags]
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
        (Provider::Claude | Provider::Codex, Event::PermissionRequest | Event::PreToolUse)
        | (Provider::Copilot, Event::PreToolUse) => {}
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
        (Provider::Codex, Event::PermissionRequest) => {
            Ok(handle_codex_permission_request(&parsed.options, input))
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

fn validate_option_support(
    provider: Provider,
    event: Event,
    options: &CliOptions,
) -> Result<(), String> {
    let mut unsupported = Vec::new();

    let supports_block_rm = matches!(
        (provider, event),
        (Provider::Claude, Event::PermissionRequest)
            | (Provider::Copilot, Event::PreToolUse)
            | (
                Provider::Codex,
                Event::PermissionRequest | Event::PreToolUse
            )
    );
    let supports_dangerous_paths = supports_block_rm;
    let supports_rust_allow = matches!(
        (provider, event),
        (
            Provider::Claude | Provider::Copilot | Provider::Codex,
            Event::PreToolUse
        )
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
