use std::borrow::Cow;
use std::fmt::Write;
use std::path::PathBuf;

use derive_setters::Setters;
use forge_api::{Effort, Usage};
use nu_ansi_term::{Color, Style};
use reedline::{Prompt, PromptHistorySearchStatus};

use crate::display_constants::markers;
use crate::utils::humanize_number;

// Constants
const MULTILINE_INDICATOR: &str = "::: ";

// Branch glyph still rendered when a git branch is detected; folder/chevron/agent
// glyphs were removed to keep the prompt font-agnostic across terminals without
// a Nerd-Font patched typeface.
const BRANCH_SYMBOL: &str = "\u{f418}"; //   branch icon

// Plain Unicode "FISHEYE" (U+25C9) — filled-circle status glyph that renders
// without requiring a Nerd-Font patched typeface.
const STATUS_DOT: &str = "\u{25c9}";

/// Terminal width at which the reasoning effort label switches from the
/// compact three-letter form (e.g. `MED`) to the full uppercase label
/// (e.g. `MEDIUM`). Matches [`crate::zsh::rprompt`] so the CLI and zsh
/// integration render identically on equivalent terminals.
const WIDE_TERMINAL_THRESHOLD: usize = 100;

/// Very Specialized Prompt for the Agent Chat
#[derive(Clone, Setters)]
#[setters(strip_option, borrow_self)]
pub struct ForgePrompt {
    pub cwd: PathBuf,
    pub usage: Option<Usage>,
    /// Currently configured reasoning effort level for the active model.
    /// `Effort::None` is suppressed (see
    /// [`ForgePrompt::render_prompt_right`]).
    pub reasoning_effort: Option<Effort>,
    pub git_branch: Option<String>,
}

impl ForgePrompt {
    /// Creates a new `ForgePrompt`, resolving the git branch once at
    /// construction time.
    pub fn new(cwd: PathBuf) -> Self {
        let git_branch = get_git_branch();
        Self {
            cwd,
            usage: None,
            reasoning_effort: None,
            git_branch,
        }
    }

    pub fn refresh(&mut self) -> &mut Self {
        let git_branch = get_git_branch();
        self.git_branch = git_branch;
        self
    }
}

impl Prompt for ForgePrompt {
    fn render_prompt_left(&self) -> Cow<'_, str> {
        // Left prompt layout:
        //
        //   dir   branch
        //
        // followed by a blank input line so the cursor sits at column 0 with no
        // Nerd-Font glyph in front of it.

        let dir_style = Style::new().fg(Color::Cyan).bold();
        let branch_style = Style::new().fg(Color::LightGreen).bold();
        let arrow_style = Style::new().fg(Color::LightGreen).bold();

        let current_dir = self
            .cwd
            .file_name()
            .and_then(|name| name.to_str())
            .map(String::from)
            .unwrap_or_else(|| markers::EMPTY.to_string());

        let mut result = String::with_capacity(80);

        // Directory name, bold cyan
        write!(result, "{}", dir_style.paint(current_dir.as_str())).unwrap();

        // Git branch — branch icon + name, bold green (only when present and
        // different from the directory name, matching existing behaviour)
        if let Some(branch) = self.git_branch.as_deref()
            && branch != current_dir
        {
            write!(
                result,
                " {}",
                branch_style.paint(format!("{BRANCH_SYMBOL} {branch}"))
            )
            .unwrap();
        }

        // Second line: a plain Unicode arrow (U+276F) that renders without
        // requiring a Nerd-Font patched typeface.
        write!(result, "\n{} ", arrow_style.paint("\u{276f}")).unwrap();

        Cow::Owned(result)
    }

    fn render_prompt_right(&self) -> Cow<'_, str> {
        // Right prompt layout: tokens · cost · `◉ <effort> · :effort`
        // Active (tokens > 0): bright white for tokens, green for cost
        // Inactive (no tokens): all segments dimmed
        // The model segment was removed — model is shown in the startup banner.

        let total_tokens = self.usage.as_ref().map(|u| u.total_tokens);
        let active = total_tokens.map(|t| *t > 0).unwrap_or(false);

        let mut result = String::with_capacity(64);

        // Token count (only shown when active)
        if let Some(tokens) = total_tokens
            && active
        {
            let prefix = match tokens {
                forge_api::TokenCount::Actual(_) => "",
                forge_api::TokenCount::Approx(_) => "~",
            };
            let count_str = format!("{}{}", prefix, humanize_number(*tokens));
            write!(
                result,
                " {}",
                Style::new().bold().fg(Color::LightGray).paint(&count_str)
            )
            .unwrap();
        }

        // Cost (only shown when active)
        if let Some(cost) = self.usage.as_ref().and_then(|u| u.cost)
            && active
        {
            let cost_str = format!("\u{f155}{cost:.2}");
            write!(
                result,
                " {}",
                Style::new().bold().fg(Color::Green).paint(&cost_str)
            )
            .unwrap();
        }

        // Reasoning effort: `◉ <effort>` with the dot color-coded by tier.
        // `Effort::None` is suppressed. On narrow terminals the label collapses
        // to its first three characters so the prompt stays compact.
        if let Some(ref effort) = self.reasoning_effort
            && !matches!(effort, Effort::None)
        {
            let value = effort_label(effort, term_width()).to_lowercase();
            let dot_color = effort_color(effort);
            write!(
                result,
                " {} {}",
                Style::new().fg(dot_color).paint(STATUS_DOT),
                Style::new().bold().fg(dot_color).paint(&value),
            )
            .unwrap();
        }

        Cow::Owned(result)
    }

    fn render_prompt_indicator(&self, _prompt_mode: reedline::PromptEditMode) -> Cow<'_, str> {
        Cow::Borrowed("")
    }

    fn render_prompt_multiline_indicator(&self) -> Cow<'_, str> {
        Cow::Borrowed(MULTILINE_INDICATOR)
    }

    fn render_prompt_history_search_indicator(
        &self,
        history_search: reedline::PromptHistorySearch,
    ) -> Cow<'_, str> {
        let prefix = match history_search.status {
            PromptHistorySearchStatus::Passing => "",
            PromptHistorySearchStatus::Failing => "failing ",
        };

        let mut result = String::with_capacity(32);

        // Handle empty search term more elegantly
        if history_search.term.is_empty() {
            write!(result, "({prefix}reverse-search) ").unwrap();
        } else {
            write!(
                result,
                "({}reverse-search: {}) ",
                prefix, history_search.term
            )
            .unwrap();
        }

        Cow::Owned(Style::new().fg(Color::White).paint(&result).to_string())
    }
}

/// Gets the current git branch name if available
fn get_git_branch() -> Option<String> {
    let repo = gix::discover(".").ok()?;
    let head = repo.head().ok()?;
    head.referent_name().map(|r| r.shorten().to_string())
}

/// Returns the current terminal width in columns, falling back to 80 when
/// the size cannot be detected.
fn term_width() -> usize {
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(80)
}

/// Maps a reasoning [`Effort`] tier to a status colour, ascending from
/// dim (minimal) to bright red (max). Mirrors a battery / temperature gauge so
/// the user can read the active tier at a glance.
fn effort_color(effort: &Effort) -> Color {
    match effort {
        Effort::None | Effort::Minimal => Color::DarkGray,
        Effort::Low => Color::Green,
        Effort::Medium => Color::Yellow,
        Effort::High => Color::Rgb(255, 165, 0), // orange
        Effort::XHigh => Color::LightRed,
        Effort::Max => Color::LightPurple,
    }
}

/// Formats an [`Effort`] as its uppercase label, collapsing to the first three
/// characters on narrow terminals (< [`WIDE_TERMINAL_THRESHOLD`] columns).
fn effort_label(effort: &Effort, width: usize) -> String {
    let full = effort.to_string().to_uppercase();
    if width >= WIDE_TERMINAL_THRESHOLD {
        full
    } else {
        // `chars().take(3)` rather than `&full[..3]` to satisfy the
        // `clippy::string_slice` lint denied in CI.
        full.chars().take(3).collect()
    }
}

#[cfg(test)]
mod tests {
    use nu_ansi_term::Style;
    use pretty_assertions::assert_eq;

    use super::*;

    impl Default for ForgePrompt {
        fn default() -> Self {
            ForgePrompt {
                cwd: PathBuf::from("."),
                usage: None,
                reasoning_effort: None,
                git_branch: None,
            }
        }
    }

    #[test]
    fn test_render_prompt_left() {
        let prompt = ForgePrompt::default();
        let actual = prompt.render_prompt_left();

        // Nerd-Font glyphs gone: no folder icon, no Nerd-Font chevron.
        assert!(!actual.contains('\u{ea83}'));
        assert!(!actual.contains('\u{f013e}'));
        // The input line uses the plain Unicode arrow followed by a space.
        assert!(actual.contains('\u{276f}'));
    }

    #[test]
    fn test_render_prompt_left_with_branch() {
        let prompt = ForgePrompt { git_branch: Some("main".to_string()), ..Default::default() };
        let actual = prompt.render_prompt_left();

        // Branch icon and name present
        assert!(actual.contains(BRANCH_SYMBOL));
        assert!(actual.contains("main"));
    }

    #[test]
    fn test_render_prompt_right_inactive() {
        // No tokens, no model, no effort → right prompt renders nothing.
        let prompt = ForgePrompt::default();

        let actual = prompt.render_prompt_right();
        // Agent/model segments are gone; nothing renders without state.
        assert!(!actual.contains('\u{f167a}'));
        assert!(!actual.to_uppercase().contains("FORGE"));
        // No model glyph appears anywhere — model lives in the banner now.
        assert!(!actual.contains('\u{ec19}'));
        // No token count text in inactive state.
        assert!(!actual.contains("1k") && !actual.contains("~"));
    }

    #[test]
    fn test_render_prompt_right_active_with_tokens() {
        // Tokens > 0 → active colours; approx tokens show "~" prefix
        let usage = Usage {
            prompt_tokens: forge_api::TokenCount::Actual(10),
            completion_tokens: forge_api::TokenCount::Actual(20),
            total_tokens: forge_api::TokenCount::Approx(30),
            ..Default::default()
        };
        let mut prompt = ForgePrompt::default();
        let _ = prompt.usage(usage);

        let actual = prompt.render_prompt_right();
        assert!(actual.contains("~30"));
        // Agent and model segments removed regardless of activity state.
        assert!(!actual.contains('\u{f167a}'));
        assert!(!actual.contains('\u{ec19}'));
    }

    #[test]
    fn test_render_prompt_multiline_indicator() {
        let prompt = ForgePrompt::default();
        let actual = prompt.render_prompt_multiline_indicator();
        let expected = MULTILINE_INDICATOR;
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_render_prompt_history_search_indicator_passing() {
        let prompt = ForgePrompt::default();
        let history_search = reedline::PromptHistorySearch {
            status: PromptHistorySearchStatus::Passing,
            term: "test".to_string(),
        };
        let actual = prompt.render_prompt_history_search_indicator(history_search);
        let expected = Style::new()
            .fg(Color::White)
            .paint("(reverse-search: test) ")
            .to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_render_prompt_history_search_indicator_failing() {
        let prompt = ForgePrompt::default();
        let history_search = reedline::PromptHistorySearch {
            status: PromptHistorySearchStatus::Failing,
            term: "test".to_string(),
        };
        let actual = prompt.render_prompt_history_search_indicator(history_search);
        let expected = Style::new()
            .fg(Color::White)
            .paint("(failing reverse-search: test) ")
            .to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_render_prompt_history_search_indicator_empty_term() {
        let prompt = ForgePrompt::default();
        let history_search = reedline::PromptHistorySearch {
            status: PromptHistorySearchStatus::Passing,
            term: "".to_string(),
        };
        let actual = prompt.render_prompt_history_search_indicator(history_search);
        let expected = Style::new()
            .fg(Color::White)
            .paint("(reverse-search) ")
            .to_string();
        assert_eq!(actual, expected);
    }

    #[test]
    fn test_render_prompt_right_with_cost() {
        // Cost shown when active
        let usage = Usage {
            total_tokens: forge_api::TokenCount::Actual(1500),
            cost: Some(0.01),
            ..Default::default()
        };
        let mut prompt = ForgePrompt::default();
        let _ = prompt.usage(usage);

        let actual = prompt.render_prompt_right();
        assert!(actual.contains("0.01"));
        assert!(actual.contains("1.5k"));
    }

    #[test]
    fn test_render_prompt_right_with_reasoning_effort() {
        // Effort renders as `◉ <effort>` with the value in lowercase. The dot
        // is color-coded by tier; no `:effort` slash hint is shown.
        let mut prompt = ForgePrompt::default();
        let _ = prompt.reasoning_effort(Effort::High);

        let actual = prompt.render_prompt_right();
        assert!(actual.contains(STATUS_DOT));
        assert!(actual.contains("high") || actual.contains("hig"));
        assert!(!actual.contains(":effort"));
    }

    #[test]
    fn test_render_prompt_right_hides_effort_none() {
        // `Effort::None` carries no useful info — nothing should be rendered
        // for the effort segment.
        let mut prompt = ForgePrompt::default();
        let _ = prompt.reasoning_effort(Effort::None);

        let actual = prompt.render_prompt_right();
        assert!(!actual.to_uppercase().contains("NONE"));
        assert!(!actual.contains(STATUS_DOT));
    }

    #[test]
    fn test_effort_label_narrow_vs_wide() {
        assert_eq!(effort_label(&Effort::Medium, 80), "MED");
        assert_eq!(
            effort_label(&Effort::Medium, WIDE_TERMINAL_THRESHOLD),
            "MEDIUM"
        );
    }
}
