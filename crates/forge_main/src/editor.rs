use std::path::PathBuf;
use std::sync::Arc;

use crossterm::event::Event;
use forge_api::Environment;
use nu_ansi_term::{Color, Style};
use reedline::{
    ColumnarMenu, DefaultHinter, EditCommand, EditMode, Emacs, FileBackedHistory, KeyCode,
    KeyModifiers, MenuBuilder, PromptEditMode, Reedline, ReedlineEvent, ReedlineMenu,
    ReedlineRawEvent, Signal, default_emacs_keybindings,
};

use super::completer::InputCompleter;
use super::zsh::paste::wrap_pasted_text;
use crate::highlighter::ForgeHighlighter;
use crate::model::ForgeCommandManager;
use crate::paste_store::PasteStore;
use crate::prompt::ForgePrompt;

// TODO: Store the last `HISTORY_CAPACITY` commands in the history file
const HISTORY_CAPACITY: usize = 1024 * 1024;
const COMPLETION_MENU: &str = "completion_menu";

pub struct ForgeEditor {
    editor: Reedline,
    paste_store: PasteStore,
}

pub enum ReadResult {
    Success(String),
    Empty,
    Continue,
    Exit,
}

impl ForgeEditor {
    fn init() -> reedline::Keybindings {
        let mut keybindings = default_emacs_keybindings();
        // on TAB press shows the completion menu, and if we've exact match it will
        // insert it
        keybindings.add_binding(
            KeyModifiers::NONE,
            KeyCode::Tab,
            ReedlineEvent::UntilFound(vec![
                ReedlineEvent::Menu(COMPLETION_MENU.to_string()),
                ReedlineEvent::Edit(vec![EditCommand::Complete]),
            ]),
        );

        // on CTRL + k press clears the screen
        keybindings.add_binding(
            KeyModifiers::CONTROL,
            KeyCode::Char('k'),
            ReedlineEvent::ClearScreen,
        );

        // on CTRL + r press searches the history
        keybindings.add_binding(
            KeyModifiers::CONTROL,
            KeyCode::Char('r'),
            ReedlineEvent::SearchHistory,
        );

        // on ALT + Enter press inserts a newline
        keybindings.add_binding(
            KeyModifiers::ALT,
            KeyCode::Enter,
            ReedlineEvent::Edit(vec![EditCommand::InsertNewline]),
        );

        keybindings
    }

    pub fn new(
        env: Environment,
        custom_history_path: Option<PathBuf>,
        manager: Arc<ForgeCommandManager>,
    ) -> Self {
        // Store file history in system config directory
        let history_file = env.history_path(custom_history_path.as_ref());

        let history = Box::new(
            FileBackedHistory::with_file(HISTORY_CAPACITY, history_file).unwrap_or_default(),
        );
        let completion_menu = Box::new(
            ColumnarMenu::default()
                .with_name(COMPLETION_MENU)
                .with_marker("")
                .with_text_style(Style::new().bold().fg(Color::Cyan))
                .with_selected_text_style(Style::new().on(Color::White).fg(Color::Black)),
        );

        let paste_store = PasteStore::new();
        let edit_mode = Box::new(ForgeEditMode::new(Self::init(), paste_store.clone()));

        let editor = Reedline::create()
            .with_completer(Box::new(InputCompleter::new(env.cwd, manager)))
            .with_history(history)
            .with_highlighter(Box::new(ForgeHighlighter))
            .with_hinter(Box::new(
                DefaultHinter::default().with_style(Style::new().fg(Color::DarkGray)),
            ))
            .with_menu(ReedlineMenu::EngineCompleter(completion_menu))
            .with_edit_mode(edit_mode)
            .with_quick_completions(true)
            .with_ansi_colors(true)
            .use_bracketed_paste(true);
        Self { editor, paste_store }
    }

    /// Shared paste store. `Console::prompt` uses this to expand
    /// `[Pasted text #N]` placeholders into the original content before
    /// dispatching the input.
    pub fn paste_store(&self) -> PasteStore {
        self.paste_store.clone()
    }

    pub fn prompt(&mut self, prompt: &mut ForgePrompt) -> anyhow::Result<ReadResult> {
        let signal = self.editor.read_line(prompt);
        prompt.refresh();
        signal
            .map(Into::into)
            .map_err(|e| anyhow::anyhow!(ReadLineError(e)))
    }

    /// Sets the buffer content to be pre-filled on the next prompt
    pub fn set_buffer(&mut self, content: String) {
        self.editor
            .run_edit_commands(&[EditCommand::InsertString(content)]);
    }
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct ReadLineError(std::io::Error);

/// Custom edit mode that wraps Emacs and intercepts paste events.
///
/// On bracketed-paste:
/// 1. If the paste is a file path (drag-and-drop), `wrap_pasted_text` turns
///    it into an `@[…]` reference and we insert that.
/// 2. Otherwise, if the paste is multi-line or large enough to clutter the
///    prompt (`PasteStore::should_placeholder`), the real text is stashed in
///    the shared `PasteStore` and we insert a short `[Pasted text #N]`
///    placeholder. `Console::prompt` expands the placeholder back to the
///    real text just before the input is submitted.
/// 3. Small single-line non-path pastes (URLs, short snippets) are inserted
///    verbatim — wrapping them would be more friction than help.
struct ForgeEditMode {
    inner: Emacs,
    paste_store: PasteStore,
}

impl ForgeEditMode {
    /// Creates a new `ForgeEditMode` wrapping an Emacs mode with the given
    /// keybindings.
    fn new(keybindings: reedline::Keybindings, paste_store: PasteStore) -> Self {
        Self { inner: Emacs::new(keybindings), paste_store }
    }
}

impl EditMode for ForgeEditMode {
    fn parse_event(&mut self, event: ReedlineRawEvent) -> ReedlineEvent {
        // Convert to the underlying crossterm event so we can inspect it
        let raw: Event = event.into();

        if let Event::Paste(ref body) = raw {
            tracing::info!(
                paste_bytes = body.len(),
                paste_lines = body.lines().count(),
                "ForgeEditMode received bracketed paste"
            );
            let wrapped = wrap_pasted_text(body);
            // If `wrap_pasted_text` rewrote the body (path wrapping kicked
            // in), insert the wrapped form verbatim. Otherwise consider
            // hiding the paste behind a placeholder.
            let insert = if wrapped != *body {
                tracing::info!("paste rewritten by wrap_pasted_text (path wrap)");
                wrapped
            } else if PasteStore::should_placeholder(body) {
                let ph = self.paste_store.insert(body.clone());
                tracing::info!(placeholder = %ph, "paste replaced with placeholder");
                ph
            } else {
                tracing::info!("paste passed through verbatim (short, no path)");
                wrapped
            };
            return ReedlineEvent::Edit(vec![EditCommand::InsertString(insert)]);
        }

        // For every other event, delegate to the inner Emacs mode.
        // We need to reconstruct a ReedlineRawEvent from the crossterm Event.
        // ReedlineRawEvent implements TryFrom<Event>.
        match ReedlineRawEvent::try_from(raw) {
            Ok(raw_event) => self.inner.parse_event(raw_event),
            Err(()) => ReedlineEvent::None,
        }
    }

    fn edit_mode(&self) -> PromptEditMode {
        self.inner.edit_mode()
    }
}

impl From<Signal> for ReadResult {
    fn from(signal: Signal) -> Self {
        match signal {
            Signal::Success(buffer) => {
                let trimmed = buffer.trim();
                if trimmed.is_empty() {
                    ReadResult::Empty
                } else {
                    ReadResult::Success(trimmed.to_string())
                }
            }
            Signal::ExternalBreak(buffer) => {
                let trimmed = buffer.trim();
                if trimmed.is_empty() {
                    ReadResult::Empty
                } else {
                    ReadResult::Success(trimmed.to_string())
                }
            }
            Signal::CtrlC => ReadResult::Continue,
            Signal::CtrlD => ReadResult::Exit,
            _ => ReadResult::Continue,
        }
    }
}
