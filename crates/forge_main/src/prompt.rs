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

// Context-utilization pie-chart glyphs. Single Unicode characters, font-
// agnostic (no Nerd Font required). The glyph fills as context fills; the
// color shifts from green → amber → red as utilization climbs.
const PIE_EMPTY: &str = "\u{25cb}"; // ○ — 0–12%
const PIE_QUARTER: &str = "\u{25d4}"; // ◔ — 12–37%
const PIE_HALF: &str = "\u{25d1}"; // ◑ — 37–62%
const PIE_THREE_QUARTERS: &str = "\u{25d5}"; // ◕ — 62–87%
const PIE_FULL: &str = "\u{25cf}"; // ● — 87–100%

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
    /// Context window of the active model in tokens. Used to render the
    /// context-utilization pie glyph in the right prompt. When `None`, the
    /// glyph is suppressed.
    pub context_window: Option<usize>,
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
            context_window: None,
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

        // Context-utilization pie glyph. The glyph fills as the context fills
        // (○ ◔ ◑ ◕ ●) and is colored green / amber / red by utilization.
        // Always rendered when the active model's context window is known —
        // a fresh session shows the empty ○ glyph so the user sees the
        // indicator from the start. A small dim percent follows the glyph so
        // on wide-context models (1M Opus) the user gets precise feedback
        // even while the pie sits in the same bucket for hundreds of K
        // tokens.
        if let Some(window) = self.context_window
            && window > 0
        {
            let used = total_tokens
                .map(|t| (*t as usize).min(window))
                .unwrap_or(0);
            let fraction = used as f64 / window as f64;
            let glyph = pie_glyph(fraction);
            let color = pie_color(fraction);
            let percent = (fraction * 100.0).round() as u32;
            write!(
                result,
                " {} {}",
                Style::new().bold().fg(color).paint(glyph),
                Style::new().dimmed().paint(format!("{percent}%")),
            )
            .unwrap();
        }

        // Reasoning effort label, color-coded by tier. `Effort::None` is
        // suppressed. On narrow terminals the label collapses to its first
        // three characters so the prompt stays compact.
        if let Some(ref effort) = self.reasoning_effort
            && !matches!(effort, Effort::None)
        {
            let value = effort_label(effort, term_width()).to_lowercase();
            let label_color = effort_color(effort);
            write!(
                result,
                " {}",
                Style::new().bold().fg(label_color).paint(&value),
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

/// Maps a context-utilization fraction (0.0–1.0+) to a 5-state pie glyph.
/// The breakpoints are equal-width quarters anchored on each glyph's centre,
/// so the transition between glyphs feels balanced.
fn pie_glyph(fraction: f64) -> &'static str {
    let f = fraction.clamp(0.0, 1.0);
    if f < 0.125 {
        PIE_EMPTY
    } else if f < 0.375 {
        PIE_QUARTER
    } else if f < 0.625 {
        PIE_HALF
    } else if f < 0.875 {
        PIE_THREE_QUARTERS
    } else {
        PIE_FULL
    }
}

/// Maps a context-utilization fraction to a green/amber/red status colour.
/// Green up to 70%, amber up to 90%, red beyond. The thresholds line up
/// with [`Agent::compaction_threshold`]'s default 90% trigger so the glyph
/// turns red just before compaction kicks in.
fn pie_color(fraction: f64) -> Color {
    let f = fraction.clamp(0.0, 1.0);
    if f < 0.7 {
        Color::LightGreen
    } else if f < 0.9 {
        Color::Rgb(255, 176, 0) // amber
    } else {
        Color::LightRed
    }
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
                context_window: None,
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
        // Effort renders as a colored lowercase label. The leading pie glyph
        // is suppressed when no token/window info is available.
        let mut prompt = ForgePrompt::default();
        let _ = prompt.reasoning_effort(Effort::High);

        let actual = prompt.render_prompt_right();
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
    }

    #[test]
    fn test_pie_glyph_states() {
        assert_eq!(pie_glyph(0.0), PIE_EMPTY);
        assert_eq!(pie_glyph(0.05), PIE_EMPTY);
        assert_eq!(pie_glyph(0.20), PIE_QUARTER);
        assert_eq!(pie_glyph(0.50), PIE_HALF);
        assert_eq!(pie_glyph(0.75), PIE_THREE_QUARTERS);
        assert_eq!(pie_glyph(0.95), PIE_FULL);
        // Clamp on overshoot.
        assert_eq!(pie_glyph(1.5), PIE_FULL);
    }

    #[test]
    fn test_pie_color_thresholds() {
        assert_eq!(pie_color(0.10), Color::LightGreen);
        assert_eq!(pie_color(0.50), Color::LightGreen);
        assert_eq!(pie_color(0.70), Color::Rgb(255, 176, 0));
        assert_eq!(pie_color(0.85), Color::Rgb(255, 176, 0));
        assert_eq!(pie_color(0.90), Color::LightRed);
        assert_eq!(pie_color(0.99), Color::LightRed);
    }

    #[test]
    fn test_render_prompt_right_shows_pie_when_window_known() {
        // 50% utilization → ◑ in amber-ish green (actually still green since
        // 0.5 < 0.7), suppressed `%` on narrow terminals but glyph present.
        let usage = Usage {
            total_tokens: forge_api::TokenCount::Actual(100_000),
            ..Default::default()
        };
        let mut prompt = ForgePrompt::default();
        let _ = prompt.usage(usage);
        let _ = prompt.context_window(200_000_usize);

        let actual = prompt.render_prompt_right();
        assert!(actual.contains(PIE_HALF), "expected ◑ glyph in {actual:?}");
    }

    #[test]
    fn test_render_prompt_right_hides_pie_when_window_unknown() {
        let usage = Usage {
            total_tokens: forge_api::TokenCount::Actual(100_000),
            ..Default::default()
        };
        let mut prompt = ForgePrompt::default();
        let _ = prompt.usage(usage);
        // context_window deliberately left None.

        let actual = prompt.render_prompt_right();
        for glyph in [PIE_EMPTY, PIE_QUARTER, PIE_HALF, PIE_THREE_QUARTERS, PIE_FULL] {
            assert!(
                !actual.contains(glyph),
                "expected no pie glyph in {actual:?}, found {glyph}"
            );
        }
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
