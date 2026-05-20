use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use forge_api::Environment;
use nu_ansi_term::{Color, Style};

use crate::editor::{ForgeEditor, ReadResult};
use crate::model::{AppCommand, ForgeCommandManager};
use crate::prompt::ForgePrompt;
use crate::tracker;

/// Console implementation for handling user input via command line.
pub struct Console {
    command: Arc<ForgeCommandManager>,
    editor: Mutex<ForgeEditor>,
}

impl Console {
    /// Creates a new instance of `Console`.
    pub fn new(
        env: Environment,
        custom_history_path: Option<PathBuf>,
        command: Arc<ForgeCommandManager>,
    ) -> Self {
        let editor = Mutex::new(ForgeEditor::new(env, custom_history_path, command.clone()));
        Self { command, editor }
    }
}

impl Console {
    pub async fn prompt(&self, prompt: &mut ForgePrompt) -> anyhow::Result<AppCommand> {
        loop {
            let mut forge_editor = self.editor.lock().unwrap();
            let user_input = forge_editor.prompt(prompt)?;

            drop(forge_editor);
            match user_input {
                ReadResult::Continue => continue,
                ReadResult::Exit => return Ok(AppCommand::Exit),
                ReadResult::Empty => continue,
                ReadResult::Success(text) => {
                    // Expand any `[Pasted text #N]` placeholders the user
                    // collected in the input field back to their real
                    // content before the command is parsed or sent to the
                    // model.
                    let paste_store = self.editor.lock().unwrap().paste_store();
                    let expanded = paste_store.expand(&text);
                    // If the expansion produced different text, the line
                    // reedline left in scrollback only shows the
                    // placeholder. Overwrite that line with the real
                    // content so the chat history reflects what the model
                    // actually received.
                    if expanded != text {
                        reprint_with_expanded(&expanded);
                    }
                    tracker::prompt(expanded.clone());
                    return self.command.parse(&expanded);
                }
            }
        }
    }

    /// Sets the buffer content for the next prompt
    pub fn set_buffer(&self, content: String) {
        let mut editor = self.editor.lock().unwrap();
        editor.set_buffer(content);
    }
}

/// Replace the input line reedline just printed (containing only the
/// `[Pasted text #N …]` placeholders) with the same prompt arrow but the
/// full expanded text. Without this the scrollback shows a placeholder for
/// the user's message and the assistant's reply has no visible context.
///
/// Implementation: emit `\x1b[1A\x1b[2K\r` to step up one line and clear it,
/// then reprint `❯ <expanded>`. Reedline always renders the input on the
/// last line of its prompt, and the placeholder fits in a single visual
/// row (it's a one-liner), so stepping up exactly one line is correct.
fn reprint_with_expanded(expanded: &str) {
    let mut stdout = std::io::stdout();
    let arrow = Style::new().fg(Color::LightGreen).bold().paint("\u{276f}");
    // \x1b[1A → move cursor up one line, \x1b[2K → clear entire line,
    // \r → carriage return so we write from column 0.
    let _ = write!(stdout, "\x1b[1A\x1b[2K\r");
    let _ = writeln!(stdout, "{} {}", arrow, expanded);
    let _ = stdout.flush();
}
