use std::io::{Stdout, Write};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    style,
    terminal::{self, disable_raw_mode, enable_raw_mode},
    ExecutableCommand, QueueableCommand,
};

use crate::completion::{self, Completion};

pub const PROMT: &str = "$ ";
pub const BELL: char = '\u{07}';
pub const NEWLINE_RAW_TERM: &str = "\r\n";

#[derive(Debug, thiserror::Error)]
pub enum ReadLineError {
    #[error("Programshutdown")]
    Shutdown(i32),
    #[error("Io Error <{0}>")]
    Io(#[from] std::io::Error),
    #[error("While handling the tab completion <{0}>")]
    TabHandling(#[from] completion::TabHandlingError),
}

pub fn read_line(
    line: &mut String,
    stdout: &mut Stdout,
    history: &[String],
) -> Result<(), ReadLineError> {
    enable_raw_mode()?;
    let res = read_line_loop(line, stdout, history);
    disable_raw_mode()?;
    res
}

fn read_line_handle_control(
    line: &mut String,
    stdout: &mut Stdout,
    code: KeyCode,
) -> Result<bool, ReadLineError> {
    match code {
        KeyCode::Char('l' | 'L') => {
            stdout
                .queue(terminal::Clear(terminal::ClearType::All))?
                .queue(cursor::MoveTo(0, 0))?
                .queue(style::Print(PROMT))?
                .queue(style::Print(&line))?;

            stdout.flush()?;
        }
        KeyCode::Char('d' | 'D') => {
            // kill program
            stdout.execute(style::Print(NEWLINE_RAW_TERM))?;

            return Err(ReadLineError::Shutdown(0));
        }
        KeyCode::Char('c' | 'C') => {
            // new line
            line.clear();

            stdout
                .queue(style::Print(NEWLINE_RAW_TERM))?
                .queue(style::Print(PROMT))?;

            stdout.flush()?;
        }
        KeyCode::Char('j' | 'J') => {
            stdout.execute(style::Print(NEWLINE_RAW_TERM))?;
            return Ok(false);
        }
        _ => {}
    }
    Ok(true)
}

fn read_line_handle_key_event(
    line: &mut String,
    stdout: &mut Stdout,
    history: &[String],
    history_idx: &mut usize,
    code: KeyCode,
) -> Result<bool, ReadLineError> {
    match code {
        KeyCode::Up => {
            if history.is_empty() {
                return Ok(true);
            }
            *history_idx = history_idx.saturating_sub(1);

            line.clear();
            line.push_str(&history[*history_idx]);
            stdout
                .queue(cursor::MoveToColumn(PROMT.len() as _))?
                .queue(terminal::Clear(terminal::ClearType::UntilNewLine))?
                .queue(style::Print(&line))?;
            stdout.flush()?;
        }
        KeyCode::Down => {
            if history.is_empty() || *history_idx == history.len() {
                return Ok(true);
            }
            *history_idx = (*history_idx + 1).min(history.len() - 1);

            line.clear();
            line.push_str(&history[*history_idx]);
            stdout
                .queue(cursor::MoveToColumn(PROMT.len() as _))?
                .queue(terminal::Clear(terminal::ClearType::UntilNewLine))?
                .queue(style::Print(&line))?;
            stdout.flush()?;
        }
        KeyCode::Enter => {
            stdout.execute(style::Print(NEWLINE_RAW_TERM))?;
            return Ok(false);
        }
        KeyCode::Backspace => {
            if line.pop().is_none() {
                return Ok(true);
            }
            stdout
                .queue(cursor::SavePosition)?
                .queue(cursor::MoveToColumn(PROMT.len() as _))?
                .queue(terminal::Clear(terminal::ClearType::UntilNewLine))?
                .queue(style::Print(&line))?
                .queue(cursor::RestorePosition)?
                .queue(cursor::MoveLeft(1))?;

            stdout.flush()?;
        }
        KeyCode::Char('\r' | '\n') => {
            stdout.execute(style::Print(NEWLINE_RAW_TERM))?;
            return Ok(false);
        }
        KeyCode::Char(c) => {
            stdout.execute(style::Print(c))?;

            line.push(c);
        }
        _ => {}
    }
    Ok(true)
}

fn read_line_loop(
    line: &mut String,
    stdout: &mut Stdout,
    history: &[String],
) -> Result<(), ReadLineError> {
    // load short hand
    let completion = completion::generate_completion()?;
    let mut tab_state = TabCompletionState::None;
    let mut history_idx = history.len();
    loop {
        match event::read()? {
            Event::Paste(s) => {
                line.push_str(&s);
            }
            Event::Key(KeyEvent {
                code,
                modifiers: KeyModifiers::CONTROL,
                ..
            }) => {
                if !read_line_handle_control(line, stdout, code)? {
                    break;
                }
            }
            Event::Key(KeyEvent {
                code: KeyCode::Tab, ..
            }) => {
                tab_state = handle_tab(stdout, line, &completion, tab_state)?;
            }
            Event::Key(KeyEvent { code, .. }) => {
                if !read_line_handle_key_event(line, stdout, history, &mut history_idx, code)? {
                    break;
                }
            }
            _ => (),
        }
    }

    Ok(())
}

#[derive(Clone, Copy)]
enum TabCompletionState {
    /// No completion required
    None,
    /// In the sate of completion
    Active,
}

fn handle_tab(
    stdout: &mut Stdout,
    line: &mut String,
    completion: &Completion,
    state: TabCompletionState,
) -> std::io::Result<TabCompletionState> {
    let matches: Vec<String> = completion.predictive_search(line.as_bytes()).collect();

    match matches.len() {
        0 => {
            stdout.execute(style::Print(BELL))?;
            return Ok(TabCompletionState::None);
        }
        1 => {
            // we found a match
            line.clear();
            line.push_str(&matches[0]);
            line.push(' ');

            stdout
                .queue(cursor::MoveToColumn(PROMT.len() as _))?
                .queue(style::Print(&line))?;

            stdout.flush()?;
            return Ok(TabCompletionState::None);
        }
        _ => {}
    }

    let prefix: Option<String> = completion.longest_prefix(line.as_bytes());

    if let Some(s) = prefix {
        if s[..] != line[..] {
            line.clear();
            line.push_str(&s);

            stdout
                .queue(cursor::MoveToColumn(PROMT.len() as _))?
                .queue(style::Print(&line))?
                .flush()?;

            stdout.flush()?;

            return Ok(TabCompletionState::Active);
        }
    }

    if let TabCompletionState::None = state {
        // ring the bell
        stdout.execute(style::Print(BELL))?;
        return Ok(TabCompletionState::Active);
    }

    stdout.queue(style::Print(NEWLINE_RAW_TERM))?;

    for option in matches {
        stdout
            .queue(style::Print(&option))?
            .queue(style::Print("  "))?;
    }

    stdout
        .queue(style::Print(NEWLINE_RAW_TERM))?
        .queue(style::Print(PROMT))?
        .queue(style::Print(line))?;

    stdout.flush()?;

    Ok(TabCompletionState::None)
}
