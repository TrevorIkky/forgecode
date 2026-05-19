use std::io;

use colored::Colorize;
use forge_tracker::VERSION;

const BANNER: &str = include_str!("banner");

/// Displays the banner with a vertical accent-bar info block.
///
/// Layout:
///
/// ```text
/// <ASCII banner>
///
/// │  forge <version>  ·  <model>
///
/// │  :new            start a new conversation
/// │  :conversations  browse saved sessions
/// │  :model          switch model
/// │  :agent          switch agent
/// │  :exit           quit  (or Ctrl+D)
/// ```
///
/// # Arguments
///
/// * `_cli_mode` - Retained for call-site compatibility; the layout is
///   identical in both interactive and one-shot CLI modes.
/// * `current_model` - Currently active model id, rendered in the status
///   header. Provider prefix is stripped (`anthropic/claude-3` → `claude-3`).
///   When `None`, the model segment is omitted.
///
/// # Environment Variables
///
/// * `FORGE_BANNER` - Optional custom banner text to display instead of the
///   default ASCII art.
pub fn display(_cli_mode: bool, current_model: Option<&str>) -> io::Result<()> {
    let banner = std::env::var("FORGE_BANNER")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| BANNER.to_string());

    print!("{banner}");

    // Vertical accent bar prefix (dimmed) shared by the status header and
    // every command row. Two trailing spaces give breathing room between the
    // bar and the content.
    let bar = "\u{2502}  ".dimmed();

    // Status header: `│  forge VERSION  ·  MODEL`
    let short_model = current_model.map(|m| m.split('/').next_back().unwrap_or(m).to_string());
    let header_text = match &short_model {
        Some(model) => format!("forge {VERSION}  \u{b7}  {model}"),
        None => format!("forge {VERSION}"),
    };
    println!();
    println!("{}{}", bar, header_text.bold());
    // Visual gap between the header and command rows — a true blank line so
    // the accent bar does not appear on a row with no actual content.
    println!();

    // Command rows. Pad the plain command string to 14 chars BEFORE applying
    // colour so ANSI escape codes don't skew column width.
    let row = |cmd: &str, desc: &str| {
        let cmd_padded = format!("{cmd:<14}");
        println!("{}{}  {}", bar, cmd_padded.cyan().bold(), desc);
    };

    row(":new", "start a new conversation");
    row(":conversations", "browse saved sessions");
    row(":model", "switch model");
    row(":agent", "switch agent");
    row(":exit", "quit  (or Ctrl+D)");
    println!();

    Ok(())
}
