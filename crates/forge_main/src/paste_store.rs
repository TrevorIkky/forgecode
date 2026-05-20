//! Bracketed-paste placeholder substitution.
//!
//! When the user pastes multi-line or large blocks of text, the raw content
//! drowns out the prompt and is awkward to edit. This module replaces those
//! pastes with a short visual placeholder (`[Pasted text #N +X lines]`) while
//! holding the real content in memory; on submit the placeholder is expanded
//! back to the original text before the message reaches the orchestrator.
//!
//! Mirrors the behaviour of Claude Code (`utils/handlePromptSubmit.ts`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use regex::Regex;

/// Minimum payload size (in characters) for a non-path paste before we
/// substitute a placeholder. Short single-line pastes (URLs, snippets) stay
/// inline; only blocks that would visually overrun the prompt get hidden.
const PASTE_PLACEHOLDER_CHAR_THRESHOLD: usize = 200;

/// Shared, thread-safe store mapping paste ids → original text.
///
/// `ForgeEditMode` writes new entries on paste; `Console::prompt` reads them
/// (via [`PasteStore::expand`]) right before the user's input is dispatched.
#[derive(Clone, Default)]
pub struct PasteStore {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Default)]
struct Inner {
    next_id: u32,
    entries: HashMap<u32, String>,
}

impl PasteStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decide whether `text` should be hidden behind a placeholder.
    ///
    /// Multi-line pastes are always hidden because they break prompt layout.
    /// Single-line pastes only become placeholders when they exceed
    /// [`PASTE_PLACEHOLDER_CHAR_THRESHOLD`] characters — that keeps URLs and
    /// short snippets visible in the input field.
    pub fn should_placeholder(text: &str) -> bool {
        let line_count = text.lines().count();
        line_count > 1 || text.chars().count() > PASTE_PLACEHOLDER_CHAR_THRESHOLD
    }

    /// Store `text` and return the placeholder to insert into the buffer.
    pub fn insert(&self, text: String) -> String {
        let line_count = text.lines().count();
        let mut inner = self.inner.lock().expect("paste store lock poisoned");
        inner.next_id += 1;
        let id = inner.next_id;
        inner.entries.insert(id, text);
        format_placeholder(id, line_count)
    }

    /// Walk `input`, replace every `[Pasted text #N]` / `[Pasted text #N +M
    /// lines]` reference with the original content. Entries that are
    /// expanded successfully are removed from the store; missing ids leave
    /// the placeholder untouched so the user can see something went wrong.
    pub fn expand(&self, input: &str) -> String {
        let re = placeholder_regex();
        if !re.is_match(input) {
            return input.to_string();
        }

        let mut inner = self.inner.lock().expect("paste store lock poisoned");
        re.replace_all(input, |caps: &regex::Captures<'_>| {
            let id_str = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            match id_str.parse::<u32>() {
                Ok(id) => match inner.entries.remove(&id) {
                    Some(real) => real,
                    None => caps.get(0).unwrap().as_str().to_string(),
                },
                Err(_) => caps.get(0).unwrap().as_str().to_string(),
            }
        })
        .into_owned()
    }
}

/// Format the visible placeholder. `line_count == 1` (or 0 for empty
/// pastes) drops the `+N lines` suffix; multi-line pastes show the count.
fn format_placeholder(id: u32, line_count: usize) -> String {
    if line_count <= 1 {
        format!("[Pasted text #{id}]")
    } else {
        format!("[Pasted text #{id} +{line_count} lines]")
    }
}

fn placeholder_regex() -> Regex {
    // Static-ish: build once per process. The pattern is cheap enough that a
    // OnceCell would be over-engineering for a UI hot path with one prompt
    // submission per turn.
    Regex::new(r"\[Pasted text #(\d+)(?: \+\d+ lines)?\]").expect("placeholder regex must compile")
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn test_should_placeholder_single_line_short() {
        assert!(!PasteStore::should_placeholder("hello"));
        assert!(!PasteStore::should_placeholder("https://example.com/path"));
    }

    #[test]
    fn test_should_placeholder_multiline() {
        assert!(PasteStore::should_placeholder("line 1\nline 2"));
    }

    #[test]
    fn test_should_placeholder_long_single_line() {
        let long = "x".repeat(PASTE_PLACEHOLDER_CHAR_THRESHOLD + 1);
        assert!(PasteStore::should_placeholder(&long));
    }

    #[test]
    fn test_insert_and_expand_single_line() {
        let store = PasteStore::new();
        let placeholder = store.insert("hello world".to_string());
        assert_eq!(placeholder, "[Pasted text #1]");
        let expanded = store.expand(&format!("look at {placeholder} please"));
        assert_eq!(expanded, "look at hello world please");
    }

    #[test]
    fn test_insert_and_expand_multiline() {
        let store = PasteStore::new();
        let placeholder = store.insert("line a\nline b\nline c".to_string());
        assert_eq!(placeholder, "[Pasted text #1 +3 lines]");
        let expanded = store.expand(placeholder.as_str());
        assert_eq!(expanded, "line a\nline b\nline c");
    }

    #[test]
    fn test_multiple_pastes_get_unique_ids() {
        let store = PasteStore::new();
        let a = store.insert("first".to_string());
        let b = store.insert("second".to_string());
        assert_eq!(a, "[Pasted text #1]");
        assert_eq!(b, "[Pasted text #2]");
        let expanded = store.expand(&format!("{a} and {b}"));
        assert_eq!(expanded, "first and second");
    }

    #[test]
    fn test_expand_leaves_unknown_placeholder_untouched() {
        let store = PasteStore::new();
        // Never inserted, so id 99 doesn't exist.
        let input = "see [Pasted text #99]";
        let expanded = store.expand(input);
        assert_eq!(expanded, input);
    }

    #[test]
    fn test_expand_consumes_entries() {
        let store = PasteStore::new();
        let placeholder = store.insert("payload".to_string());
        let _first = store.expand(&placeholder);
        // Second expand can't find the entry any more.
        let second = store.expand(&placeholder);
        assert_eq!(second, placeholder);
    }
}
