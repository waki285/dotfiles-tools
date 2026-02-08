use serde::Deserialize;
use std::{
    env,
    fmt::Write as _,
    io::{self, Read},
    process::ExitCode,
};
use terminal_size::{Width, terminal_size};

#[derive(Debug, Deserialize)]
struct StatusInput {
    #[serde(rename = "hook_event_name")]
    _event_name: Option<String>,
    cwd: Option<String>,
    model: Option<ModelInfo>,
    workspace: Option<WorkspaceInfo>,
    version: Option<String>,
    context_window: Option<ContextWindow>,
}

#[derive(Debug, Deserialize)]
struct ModelInfo {
    id: Option<String>,
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkspaceInfo {
    current_dir: Option<String>,
    project_dir: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContextWindow {
    total_input_tokens: Option<u64>,
    total_output_tokens: Option<u64>,
    #[serde(rename = "context_window_size")]
    window_size: Option<u64>,
    current_usage: Option<CurrentUsage>,
}

#[derive(Debug, Deserialize)]
struct CurrentUsage {
    #[serde(rename = "input_tokens")]
    input: Option<u64>,
    #[serde(rename = "output_tokens")]
    output: Option<u64>,
    #[serde(rename = "cache_creation_input_tokens")]
    cache_creation_input: Option<u64>,
    #[serde(rename = "cache_read_input_tokens")]
    cache_read_input: Option<u64>,
}

#[derive(Clone, Copy)]
struct Color(u8, u8, u8);

struct Segment {
    text: String,
    fg: Color,
    bg: Color,
}

const POWERLINE_ARROW: char = '\u{e0b0}';

fn main() -> ExitCode {
    let mut stdin = String::new();
    if let Err(err) = io::stdin().read_to_string(&mut stdin) {
        eprintln!("failed to read stdin: {err}");
        return ExitCode::FAILURE;
    }

    if stdin.trim().is_empty() {
        return ExitCode::SUCCESS;
    }

    let input: StatusInput = match serde_json::from_str(&stdin) {
        Ok(input) => input,
        Err(err) => {
            eprintln!("failed to parse status json: {err}");
            return ExitCode::FAILURE;
        }
    };

    println!("{}", build_statusline(&input));
    ExitCode::SUCCESS
}

fn build_statusline(input: &StatusInput) -> String {
    let model = input
        .model
        .as_ref()
        .and_then(|model| model.display_name.as_deref().or(model.id.as_deref()))
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");

    let cwd = input
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.current_dir.as_deref())
        .or(input.cwd.as_deref())
        .unwrap_or(".");

    let project_dir = input
        .workspace
        .as_ref()
        .and_then(|workspace| workspace.project_dir.as_deref());

    let mut left_segments = vec![
        Segment {
            text: format!("model {model}"),
            fg: Color(245, 240, 255),
            bg: Color(146, 72, 177),
        },
        Segment {
            text: format!("cwd {}", shorten_path(cwd, 3)),
            fg: Color(255, 235, 244),
            bg: Color(238, 96, 146),
        },
    ];

    if let Some(project_dir) = project_dir
        && project_dir != cwd
    {
        left_segments.push(Segment {
            text: format!("project {}", shorten_path(project_dir, 2)),
            fg: Color(255, 243, 234),
            bg: Color(242, 149, 108),
        });
    }

    if let Some(percent) = context_usage_percent(input) {
        left_segments.push(Segment {
            text: format!("ctx {percent:.1}%"),
            fg: Color(233, 247, 255),
            bg: Color(67, 156, 205),
        });
    }

    let (left_styled, left_width) = render_powerline(&left_segments);

    let version_text = input
        .version
        .as_deref()
        .filter(|version| !version.is_empty())
        .map_or_else(|| "vunknown".to_string(), normalized_version);

    let right_label = format!(" {version_text} ");
    let right_width = visible_width(&right_label);
    let right_styled = format!(
        "{}{}{}\x1b[0m",
        bg(Color(48, 120, 168)),
        fg(Color(233, 245, 255)),
        right_label
    );

    let spacing = terminal_size()
        .map(|(Width(width), _)| usize::from(width))
        .and_then(|width| width.checked_sub(left_width + right_width))
        .unwrap_or(1)
        .max(1);

    format!("{left_styled}{}{right_styled}", " ".repeat(spacing))
}

fn normalized_version(version: &str) -> String {
    if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{version}")
    }
}

fn context_usage_percent(input: &StatusInput) -> Option<f64> {
    let context = input.context_window.as_ref()?;
    let window_size = context.window_size?;
    if window_size == 0 {
        return None;
    }

    let used_tokens = context.current_usage.as_ref().map_or_else(
        || {
            context
                .total_input_tokens
                .unwrap_or(0)
                .saturating_add(context.total_output_tokens.unwrap_or(0))
        },
        |current_usage| {
            current_usage
                .input
                .unwrap_or(0)
                .saturating_add(current_usage.output.unwrap_or(0))
                .saturating_add(current_usage.cache_creation_input.unwrap_or(0))
                .saturating_add(current_usage.cache_read_input.unwrap_or(0))
        },
    );

    let used_tokens = u32::try_from(used_tokens).unwrap_or(u32::MAX);
    let window_size = u32::try_from(window_size).unwrap_or(u32::MAX);

    Some(f64::from(used_tokens) * 100.0 / f64::from(window_size))
}

fn shorten_path(path: &str, keep_components: usize) -> String {
    if path.is_empty() {
        return ".".to_string();
    }

    let normalized = replace_home_prefix(path);

    if normalized == "/" || normalized == "~" {
        return normalized;
    }

    let mut components: Vec<&str> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();

    if components.first().is_some_and(|part| *part == "~") {
        components.remove(0);
    }

    if components.len() <= keep_components {
        return normalized;
    }

    let tail = components[components.len() - keep_components..].join("/");
    format!("…/{tail}")
}

fn replace_home_prefix(path: &str) -> String {
    let Some(home) = env::var_os("HOME") else {
        return path.to_string();
    };

    let home = home.to_string_lossy();
    if path == home {
        "~".to_string()
    } else if let Some(rest) = path.strip_prefix(&format!("{home}/")) {
        format!("~/{rest}")
    } else {
        path.to_string()
    }
}

fn render_powerline(segments: &[Segment]) -> (String, usize) {
    if segments.is_empty() {
        return (String::new(), 0);
    }

    let mut rendered = String::new();
    let mut width = 0usize;

    for (idx, segment) in segments.iter().enumerate() {
        write!(
            rendered,
            "{}{} {} \x1b[0m",
            bg(segment.bg),
            fg(segment.fg),
            segment.text
        )
        .expect("writing into String must succeed");
        width += visible_width(&segment.text) + 2;

        if let Some(next) = segments.get(idx + 1) {
            write!(
                rendered,
                "{}{}{}\x1b[0m",
                fg(segment.bg),
                bg(next.bg),
                POWERLINE_ARROW
            )
            .expect("writing into String must succeed");
        } else {
            write!(rendered, "{}{}\x1b[0m", fg(segment.bg), POWERLINE_ARROW)
                .expect("writing into String must succeed");
        }
        width += 1;
    }

    (rendered, width)
}

fn visible_width(text: &str) -> usize {
    text.chars().count()
}

fn fg(color: Color) -> String {
    format!("\x1b[38;2;{};{};{}m", color.0, color.1, color.2)
}

fn bg(color: Color) -> String {
    format!("\x1b[48;2;{};{};{}m", color.0, color.1, color.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_prefix_is_normalized() {
        assert_eq!(normalized_version("1.2.3"), "v1.2.3");
        assert_eq!(normalized_version("v1.2.3"), "v1.2.3");
    }

    #[test]
    fn context_usage_prefers_current_usage() {
        let input = StatusInput {
            _event_name: None,
            cwd: None,
            model: None,
            workspace: None,
            version: None,
            context_window: Some(ContextWindow {
                total_input_tokens: Some(1),
                total_output_tokens: Some(1),
                window_size: Some(100),
                current_usage: Some(CurrentUsage {
                    input: Some(20),
                    output: Some(5),
                    cache_creation_input: Some(10),
                    cache_read_input: Some(15),
                }),
            }),
        };

        let percent = context_usage_percent(&input).unwrap_or_default();
        assert!((percent - 50.0).abs() < 0.0001);
    }

    #[test]
    fn path_is_shortened_from_head() {
        assert_eq!(
            shorten_path("/Users/alice/work/project/src/bin", 2),
            "…/src/bin"
        );
    }
}
