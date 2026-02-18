use crossterm::style::{Color, ResetColor, SetBackgroundColor, SetForegroundColor};
use serde::Deserialize;
use std::{
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
    #[expect(dead_code)]
    version: Option<String>,
    cost: Option<CostInfo>,
    context_window: Option<ContextWindow>,
}

#[derive(Debug, Deserialize)]
struct CostInfo {
    total_cost_usd: Option<f64>,
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
    #[expect(dead_code)]
    total_input_tokens: Option<u64>,
    #[expect(dead_code)]
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
const CONTEXT_BAR_SLOTS: usize = 10;
const CONTEXT_BAR_FILLED: char = '█';
const CONTEXT_BAR_EMPTY: char = '░';
const CONTEXT_BAR_THRESHOLDS: [f64; CONTEXT_BAR_SLOTS] =
    [10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0];

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
    let raw_model = input
        .model
        .as_ref()
        .and_then(|value| value.display_name.as_deref().or(value.id.as_deref()))
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");
    let model = prettify_model_name(raw_model);

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
            text: format!("\u{f4b8} {model}"),
            fg: rgb(245, 240, 255),
            bg: rgb(146, 72, 177),
        },
        Segment {
            text: format!("\u{f07c} {}", folder_name(cwd)),
            fg: rgb(255, 235, 244),
            bg: rgb(238, 96, 146),
        },
    ];

    if let Some(project_dir) = project_dir
        && project_dir != cwd
    {
        left_segments.push(Segment {
            text: format!("\u{e5fb} {}", folder_name(project_dir)),
            fg: rgb(255, 243, 234),
            bg: rgb(242, 149, 108),
        });
    }

    if let Some(git_ref) = git_ref_for_dir(git_lookup_dir) {
        left_segments.push(Segment {
            text: format!("\u{e725} {git_ref}"),
            fg: rgb(232, 247, 239),
            bg: rgb(72, 153, 120),
        });
    }

    if let Some(cost_label) = format_cost(input) {
        left_segments.push(Segment {
            text: cost_label,
            fg: rgb(235, 245, 255),
            bg: rgb(48, 120, 168),
        });
    }

    if let Some(percent) = context_usage_percent(input) {
        let (text_color, fill_color) = context_segment_colors(percent);
        left_segments.push(Segment {
            text: context_usage_label(percent),
            fg: text_color,
            bg: fill_color,
        });
    }

    let (left_styled, _left_width) = render_powerline(&left_segments);

    left_styled
}

/// Transform a raw model ID into a human-friendly display name.
///
/// Examples:
///   `ag/claude-opus-4-6-thinking`      -> `Opus 4.6 🧠`
///   `ag/claude-opus-4-6-thinking[1m]`  -> `Opus 4.6 (1M) 🧠`
///   `ag/claude-sonnet-4-5-thinking`    -> `Sonnet 4.5 🧠`
///   `ag/gemini-2.5-flash-lite[1m]`     -> `Gemini 2.5 Flash Lite (1M)`
///   `ag/gemini-2.5-pro`                -> `Gemini 2.5 Pro 🧠`
///   `claude-opus-4.5`                  -> `Opus 4.5`
///   `v/gpt-5.3-codex(xhigh)`          -> `GPT 5.3 Codex (xhigh) 🧠`
///   `gpt-4.1-2025-04-14`              -> `GPT 4.1`
///   `unknown-model`                    -> `unknown-model`
fn prettify_model_name(raw: &str) -> String {
    let (body, qualifier) = extract_qualifier(raw);

    // Strip routing prefixes: "ag/", "v/"
    let body = body
        .strip_prefix("ag/")
        .or_else(|| body.strip_prefix("v/"))
        .unwrap_or(body);

    let is_thinking = body.ends_with("-thinking");
    let body = body.strip_suffix("-thinking").unwrap_or(body);

    let (pretty, is_reasoning) = if let Some(rest) = body.strip_prefix("claude-") {
        (prettify_claude(rest), is_thinking)
    } else if let Some(rest) = body.strip_prefix("gemini-") {
        let parts: Vec<&str> = rest.split('-').collect();
        let is_pro = parts.iter().any(|p| p.eq_ignore_ascii_case("pro"));
        (prettify_generic("Gemini", rest), is_pro)
    } else if let Some(rest) = body.strip_prefix("gpt-") {
        let reasoning = is_gpt_reasoning(rest, qualifier.as_deref());
        (prettify_generic("GPT", rest), reasoning)
    } else {
        return raw.to_string();
    };

    let mut result = match qualifier {
        Some(q) => format!("{pretty} ({q})"),
        None => pretty,
    };

    if is_reasoning {
        result.push_str(" 🧠");
    }

    result
}

/// Determine if a GPT model qualifies as a reasoning model.
///
/// Rules:
/// - GPT Codex (non-mini) with medium+ reasoning qualifier -> true
/// - GPT 5+ (non-mini) -> true
fn is_gpt_reasoning(rest: &str, qualifier: Option<&str>) -> bool {
    let parts: Vec<&str> = rest.split('-').collect();
    if parts.iter().any(|p| p.eq_ignore_ascii_case("mini")) {
        return false;
    }

    let is_codex = parts.iter().any(|p| p.eq_ignore_ascii_case("codex"));
    if is_codex {
        return qualifier.is_some_and(|q| {
            let q_lower = q.to_ascii_lowercase();
            q_lower == "medium" || q_lower == "high" || q_lower == "xhigh"
        });
    }

    // GPT 5+ (non-mini)
    parts
        .first()
        .and_then(|v| v.split('.').next())
        .and_then(|major| major.parse::<u32>().ok())
        .is_some_and(|major| major >= 5)
}

/// Extract a trailing qualifier from a model ID.
/// Handles both `[...]` and `(...)` syntax.
/// Returns `(body, Some(inner))` or `(original, None)`.
fn extract_qualifier(raw: &str) -> (&str, Option<String>) {
    if let Some(start) = raw.rfind('[')
        && raw.ends_with(']')
    {
        let inner = &raw[start + 1..raw.len() - 1];
        return (&raw[..start], Some(inner.to_uppercase()));
    }
    if let Some(start) = raw.rfind('(')
        && raw.ends_with(')')
    {
        let inner = &raw[start + 1..raw.len() - 1];
        return (&raw[..start], Some(inner.to_string()));
    }
    (raw, None)
}

/// Prettify a Claude model name after "claude-" prefix is stripped.
/// e.g. "opus-4-6" -> "Opus 4.6", "sonnet-4-5" -> "Sonnet 4.5"
fn prettify_claude(rest: &str) -> String {
    let parts: Vec<&str> = rest.splitn(2, '-').collect();
    if parts.len() < 2 {
        return title_case(rest);
    }

    let tier = title_case(parts[0]);
    let version = dotted_version(parts[1]);
    format!("{tier} {version}")
}

/// Prettify a non-Claude model (Gemini, GPT) after the prefix is stripped.
/// e.g. brand="GPT", rest="5.3-codex" -> "GPT 5.3 Codex"
/// e.g. brand="Gemini", rest="2.5-flash-lite" -> "Gemini 2.5 Flash Lite"
/// e.g. brand="GPT", rest="4.1-2025-04-14" -> "GPT 4.1"
fn prettify_generic(brand: &str, rest: &str) -> String {
    let (version, name_parts) = split_version_and_name(rest);
    if name_parts.is_empty() {
        format!("{brand} {version}")
    } else {
        let name = name_parts
            .iter()
            .map(|p| title_case(p))
            .collect::<Vec<_>>()
            .join(" ");
        format!("{brand} {version} {name}")
    }
}

/// Split version and name segments from a model suffix.
///
/// - `5.3-codex` -> `("5.3", ["codex"])`
/// - `2.5-flash-lite` -> `("2.5", ["flash", "lite"])`
/// - `4.1-2025-04-14` -> `("4.1", [])` (date suffixes are dropped)
fn split_version_and_name(rest: &str) -> (String, Vec<&str>) {
    let parts: Vec<&str> = rest.split('-').collect();
    if parts.is_empty() {
        return (rest.to_string(), vec![]);
    }

    let version = parts[0].to_string();
    let remaining = &parts[1..];

    // Drop date suffixes (YYYY-MM-DD pattern)
    if remaining.len() >= 3
        && remaining[0].len() == 4
        && remaining[0].chars().all(|c| c.is_ascii_digit())
        && remaining[1].len() == 2
        && remaining[2].len() == 2
    {
        return (version, vec![]);
    }

    (version, remaining.to_vec())
}

/// Convert a hyphenated version like "4-6" to "4.6".
/// If it already contains dots (e.g. "4.5"), return as-is.
fn dotted_version(version: &str) -> String {
    if version.contains('.') {
        return version.to_string();
    }
    version.replace('-', ".")
}

fn title_case(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| {
        let upper: String = first.to_uppercase().collect();
        format!("{upper}{}", chars.as_str())
    })
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

fn context_usage_percent(input: &StatusInput) -> Option<f64> {
    let context = input.context_window.as_ref()?;
    let window_size = context.window_size?;
    if window_size == 0 {
        return None;
    }

    // Only use current_usage for context window calculation.
    // total_input_tokens / total_output_tokens are session-wide cumulative totals
    // that persist across /clear and plan mode resets, which would produce absurd
    // percentages (e.g. 6375%) when used against the (reset) window size.
    let current_usage = context.current_usage.as_ref()?;
    let used_tokens = current_usage
        .input
        .unwrap_or(0)
        .saturating_add(current_usage.output.unwrap_or(0))
        .saturating_add(current_usage.cache_creation_input.unwrap_or(0))
        .saturating_add(current_usage.cache_read_input.unwrap_or(0));

    // During early streaming, Claude Code may send current_usage with all
    // token counts at zero. Treat this the same as absent to avoid briefly
    // flashing "0.0%" in the statusline.
    if used_tokens == 0 {
        return None;
    }

    let used_tokens = u32::try_from(used_tokens).unwrap_or(u32::MAX);
    let window_size = u32::try_from(window_size).unwrap_or(u32::MAX);

    Some(f64::from(used_tokens) * 100.0 / f64::from(window_size))
}

fn format_cost(input: &StatusInput) -> Option<String> {
    let cost = input.cost.as_ref()?.total_cost_usd?;
    if cost <= 0.0 {
        return None;
    }
    Some(format!("$ {cost:.2}"))
}

fn context_segment_colors(percent: f64) -> (Color, Color) {
    if percent > 75.0 {
        (rgb(255, 242, 242), rgb(197, 66, 68))
    } else if percent > 50.0 {
        (rgb(41, 28, 0), rgb(232, 186, 77))
    } else {
        (rgb(233, 247, 255), rgb(67, 156, 205))
    }
}

fn context_usage_label(percent: f64) -> String {
    let clamped_percent = percent.clamp(0.0, 100.0);
    let filled_slots = CONTEXT_BAR_THRESHOLDS
        .iter()
        .filter(|&&threshold| clamped_percent >= threshold)
        .count();
    let empty_slots = CONTEXT_BAR_SLOTS.saturating_sub(filled_slots);
    let bar = format!(
        "{}{}",
        CONTEXT_BAR_FILLED.to_string().repeat(filled_slots),
        CONTEXT_BAR_EMPTY.to_string().repeat(empty_slots)
    );
    format!("󰆼 [{bar}] {percent:.1}%")
}

fn folder_name(path: &str) -> String {
    if path.is_empty() {
        return ".".to_string();
    }

    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return "/".to_string();
    }

    trimmed
        .rsplit(['/', '\\'])
        .find(|part| !part.is_empty())
        .map_or_else(|| ".".to_string(), ToString::to_string)
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
    fn context_usage_prefers_current_usage() {
        let input = StatusInput {
            _event_name: None,
            cwd: None,
            model: None,
            workspace: None,
            version: None,
            cost: None,
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
    fn context_usage_none_without_current_usage() {
        // After /clear or plan mode reset, current_usage may be absent while
        // total_input/output_tokens still carry stale cumulative values.
        // We must NOT fall back to those totals.
        let input = StatusInput {
            _event_name: None,
            cwd: None,
            model: None,
            workspace: None,
            version: None,
            cost: None,
            context_window: Some(ContextWindow {
                total_input_tokens: Some(500_000),
                total_output_tokens: Some(200_000),
                window_size: Some(200_000),
                current_usage: None,
            }),
        };

        assert!(context_usage_percent(&input).is_none());
    }

    #[test]
    fn context_usage_none_when_all_tokens_zero() {
        // During early streaming, current_usage may be present but all
        // token counts are zero. This should be treated as absent to
        // avoid briefly flashing "0.0%".
        let input = StatusInput {
            _event_name: None,
            cwd: None,
            model: None,
            workspace: None,
            version: None,
            cost: None,
            context_window: Some(ContextWindow {
                total_input_tokens: None,
                total_output_tokens: None,
                window_size: Some(200_000),
                current_usage: Some(CurrentUsage {
                    input: Some(0),
                    output: Some(0),
                    cache_creation_input: Some(0),
                    cache_read_input: Some(0),
                }),
            }),
        };

        assert!(context_usage_percent(&input).is_none());
    }

    #[test]
    fn context_colors_change_at_thresholds() {
        assert_eq!(
            context_segment_colors(50.0),
            (rgb(233, 247, 255), rgb(67, 156, 205))
        );
        assert_eq!(
            context_segment_colors(50.1),
            (rgb(41, 28, 0), rgb(232, 186, 77))
        );
        assert_eq!(
            context_segment_colors(75.0),
            (rgb(41, 28, 0), rgb(232, 186, 77))
        );
        assert_eq!(
            context_segment_colors(75.1),
            (rgb(255, 242, 242), rgb(197, 66, 68))
        );
    }

    #[test]
    fn context_usage_label_displays_progress_bar() {
        assert_eq!(context_usage_label(0.0), "󰆼 [░░░░░░░░░░] 0.0%");
        assert_eq!(context_usage_label(50.0), "󰆼 [█████░░░░░] 50.0%");
        assert_eq!(context_usage_label(87.3), "󰆼 [████████░░] 87.3%");
        assert_eq!(context_usage_label(120.0), "󰆼 [██████████] 120.0%");
    }

    #[test]
    fn folder_name_is_extracted() {
        assert_eq!(folder_name("/Users/alice/work/project/src/bin"), "bin");
        assert_eq!(folder_name("/tmp/"), "tmp");
        assert_eq!(folder_name("/"), "/");
        assert_eq!(folder_name(r"V:\Projects\ctf"), "ctf");
        assert_eq!(folder_name(r"C:\Users\alice\work"), "work");
        assert_eq!(folder_name(r"C:\Users\alice\work\"), "work");
    }

    #[test]
    fn truncate_preserves_width_limit() {
        let original = "feature/very-long-branch-name-for-statusline";
        let truncated = truncate_to_width(original, 16);
        assert!(visible_width(&truncated) <= 16);
        assert!(truncated.ends_with('…'));
    }

    #[test]
    fn prettify_claude_opus() {
        assert_eq!(
            prettify_model_name("ag/claude-opus-4-6-thinking"),
            "Opus 4.6 🧠"
        );
    }

    #[test]
    fn prettify_claude_opus_with_context() {
        assert_eq!(
            prettify_model_name("ag/claude-opus-4-6-thinking[1m]"),
            "Opus 4.6 (1M) 🧠"
        );
    }

    #[test]
    fn prettify_claude_sonnet() {
        assert_eq!(
            prettify_model_name("ag/claude-sonnet-4-5-thinking"),
            "Sonnet 4.5 🧠"
        );
    }

    #[test]
    fn prettify_claude_without_thinking() {
        assert_eq!(prettify_model_name("claude-opus-4.5"), "Opus 4.5");
        assert_eq!(prettify_model_name("claude-sonnet-4.5"), "Sonnet 4.5");
    }

    #[test]
    fn prettify_gemini_pro_is_reasoning() {
        assert_eq!(
            prettify_model_name("ag/gemini-2.5-pro"),
            "Gemini 2.5 Pro 🧠"
        );
    }

    #[test]
    fn prettify_gemini_flash_is_not_reasoning() {
        assert_eq!(
            prettify_model_name("ag/gemini-2.5-flash-lite[1m]"),
            "Gemini 2.5 Flash Lite (1M)"
        );
    }

    #[test]
    fn prettify_gpt_codex_with_reasoning() {
        assert_eq!(
            prettify_model_name("v/gpt-5.3-codex(xhigh)"),
            "GPT 5.3 Codex (xhigh) 🧠"
        );
        assert_eq!(
            prettify_model_name("gpt-5.3-codex(high)"),
            "GPT 5.3 Codex (high) 🧠"
        );
        assert_eq!(
            prettify_model_name("gpt-5.3-codex(medium)"),
            "GPT 5.3 Codex (medium) 🧠"
        );
    }

    #[test]
    fn prettify_gpt_codex_low_reasoning_is_not_thinking() {
        assert_eq!(
            prettify_model_name("gpt-5.3-codex(low)"),
            "GPT 5.3 Codex (low)"
        );
    }

    #[test]
    fn prettify_gpt_codex_mini_is_not_reasoning() {
        assert_eq!(
            prettify_model_name("gpt-5.3-codex-mini(high)"),
            "GPT 5.3 Codex Mini (high)"
        );
    }

    #[test]
    fn prettify_gpt5_is_reasoning() {
        assert_eq!(prettify_model_name("gpt-5"), "GPT 5 🧠");
        assert_eq!(prettify_model_name("gpt-5.1"), "GPT 5.1 🧠");
    }

    #[test]
    fn prettify_gpt4_is_not_reasoning() {
        assert_eq!(prettify_model_name("gpt-4.1-2025-04-14"), "GPT 4.1");
    }

    #[test]
    fn prettify_gpt5_mini_is_not_reasoning() {
        assert_eq!(prettify_model_name("gpt-5-mini"), "GPT 5 Mini");
    }

    #[test]
    fn prettify_unknown_passthrough() {
        assert_eq!(prettify_model_name("unknown"), "unknown");
        assert_eq!(
            prettify_model_name("some-custom-model"),
            "some-custom-model"
        );
    }

    fn make_input_with_cost(cost: Option<f64>) -> StatusInput {
        StatusInput {
            _event_name: None,
            cwd: None,
            model: None,
            workspace: None,
            version: None,
            cost: cost.map(|c| CostInfo {
                total_cost_usd: Some(c),
            }),
            context_window: None,
        }
    }

    #[test]
    fn format_cost_displays_usd() {
        let input = make_input_with_cost(Some(1.234));
        assert_eq!(format_cost(&input).unwrap(), "$ 1.23");
    }

    #[test]
    fn format_cost_zero_returns_none() {
        let input = make_input_with_cost(Some(0.0));
        assert!(format_cost(&input).is_none());
    }

    #[test]
    fn format_cost_none_returns_none() {
        let input = make_input_with_cost(None);
        assert!(format_cost(&input).is_none());
    }
}
