use crossterm::{
    cursor::{MoveLeft, MoveRight},
    style::{Color, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal,
};
use serde::Deserialize;
use std::{
    env,
    fmt::Write as _,
    io::{self, Read},
    process::Command,
    process::ExitCode,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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

struct Segment {
    text: String,
    fg: Color,
    bg: Color,
}

const POWERLINE_ARROW: char = '\u{e0b0}';
const LEADING_COLORED_PADDING: &str = "  ";
const RIGHT_EDGE_JUMP: u16 = 9999;

fn main() -> ExitCode {
    crossterm::style::force_color_output(true);

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
        .and_then(|value| value.display_name.as_deref().or(value.id.as_deref()))
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
    let git_lookup_dir = project_dir.unwrap_or(cwd);

    let mut left_segments = vec![
        Segment {
            text: format!(" {model}"),
            fg: rgb(245, 240, 255),
            bg: rgb(146, 72, 177),
        },
        Segment {
            text: format!(" {}", shorten_path(cwd, 3)),
            fg: rgb(255, 235, 244),
            bg: rgb(238, 96, 146),
        },
    ];

    if let Some(project_dir) = project_dir
        && project_dir != cwd
    {
        left_segments.push(Segment {
            text: format!(" {}", shorten_path(project_dir, 2)),
            fg: rgb(255, 243, 234),
            bg: rgb(242, 149, 108),
        });
    }

    if let Some(git_ref) = git_ref_for_dir(git_lookup_dir) {
        left_segments.push(Segment {
            text: format!(" {git_ref}"),
            fg: rgb(232, 247, 239),
            bg: rgb(72, 153, 120),
        });
    }

    if let Some(percent) = context_usage_percent(input) {
        left_segments.push(Segment {
            text: format!("󰆼 {percent:.1}%"),
            fg: rgb(233, 247, 255),
            bg: rgb(67, 156, 205),
        });
    }

    let (left_styled, left_width) = render_powerline(&left_segments);
    let (left_prefix, left_prefix_width) = left_segments.first().map_or_else(
        || (String::new(), 0),
        |segment| {
            (
                format!(
                    "{}{}",
                    SetBackgroundColor(segment.bg),
                    LEADING_COLORED_PADDING
                ),
                visible_width(LEADING_COLORED_PADDING),
            )
        },
    );

    let version_text = input
        .version
        .as_deref()
        .filter(|value| !value.is_empty())
        .map_or_else(|| "vunknown".to_string(), normalized_version);

    let right_label = format!(" 󰎙 {version_text} ");
    let right_width = visible_width(&right_label);
    let right_styled = format!(
        "{}{}{}{}",
        SetBackgroundColor(rgb(48, 120, 168)),
        SetForegroundColor(rgb(233, 245, 255)),
        right_label,
        ResetColor
    );

    let required_width = left_prefix_width + left_width + right_width;
    if let Some(terminal_width) = terminal_columns()
        && terminal_width > required_width
    {
        let spacing = terminal_width - required_width;
        return format!(
            "{left_prefix}{left_styled}{}{right_styled}",
            " ".repeat(spacing)
        );
    }

    let move_left = u16::try_from(right_width).unwrap_or(u16::MAX);
    format!(
        "{left_prefix}{left_styled}{}{}{right_styled}",
        MoveRight(RIGHT_EDGE_JUMP),
        MoveLeft(move_left)
    )
}

fn terminal_columns() -> Option<usize> {
    if let Ok((columns, _)) = terminal::size() {
        return Some(usize::from(columns));
    }
    env::var("COLUMNS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
}

fn git_ref_for_dir(dir: &str) -> Option<String> {
    git_command_output(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .or_else(|| git_command_output(dir, &["rev-parse", "--short", "HEAD"]))
        .map(|value| truncate_to_width(&value, 28))
}

fn git_command_output(dir: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn truncate_to_width(value: &str, max_width: usize) -> String {
    if visible_width(value) <= max_width {
        return value.to_string();
    }

    if max_width == 0 {
        return String::new();
    }

    let ellipsis = "…";
    let ellipsis_width = visible_width(ellipsis);
    if max_width <= ellipsis_width {
        return ellipsis.to_string();
    }

    let mut result = String::new();
    let mut width = 0usize;
    for ch in value.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width + ellipsis_width > max_width {
            break;
        }
        result.push(ch);
        width += ch_width;
    }

    result.push('…');
    result
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

    let arrow_width = UnicodeWidthChar::width(POWERLINE_ARROW).unwrap_or(1);
    let mut rendered = String::new();
    let mut width = 0usize;

    for (idx, segment) in segments.iter().enumerate() {
        write!(
            rendered,
            "{}{} {} {}",
            SetBackgroundColor(segment.bg),
            SetForegroundColor(segment.fg),
            segment.text,
            ResetColor
        )
        .expect("writing into String must succeed");
        width += visible_width(&segment.text) + 2;

        if let Some(next) = segments.get(idx + 1) {
            write!(
                rendered,
                "{}{}{}{}",
                SetForegroundColor(segment.bg),
                SetBackgroundColor(next.bg),
                POWERLINE_ARROW,
                ResetColor
            )
            .expect("writing into String must succeed");
        } else {
            write!(
                rendered,
                "{}{}{}",
                SetForegroundColor(segment.bg),
                POWERLINE_ARROW,
                ResetColor
            )
            .expect("writing into String must succeed");
        }

        width += arrow_width;
    }

    (rendered, width)
}

fn visible_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb { r, g, b }
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

    #[test]
    fn leading_padding_is_colored() {
        let input = StatusInput {
            _event_name: None,
            cwd: None,
            model: None,
            workspace: None,
            version: None,
            context_window: None,
        };
        let line = build_statusline(&input);
        let expected_prefix = format!(
            "{}{}",
            SetBackgroundColor(rgb(146, 72, 177)),
            LEADING_COLORED_PADDING
        );
        assert!(line.starts_with(&expected_prefix));
    }

    #[test]
    fn truncate_preserves_width_limit() {
        let original = "feature/very-long-branch-name-for-statusline";
        let truncated = truncate_to_width(original, 16);
        assert!(visible_width(&truncated) <= 16);
        assert!(truncated.ends_with('…'));
    }
}
