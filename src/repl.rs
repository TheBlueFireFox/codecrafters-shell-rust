use std::{
    borrow::Cow,
    ffi::OsStr,
    fs::File,
    io::{Stderr, Stdout, Write},
    path::PathBuf,
    process::{Command, Stdio},
    str::FromStr,
};

use anyhow::Context as _;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    style,
    terminal::{self, disable_raw_mode, enable_raw_mode},
    ExecutableCommand, QueueableCommand,
};
use itertools::Itertools;

use crate::args;

pub type ExitCode = i32;

#[derive(thiserror::Error, Debug)]
enum Errors<'name> {
    #[error("exit code called {0}")]
    ExitCode(ExitCode),
    #[error("shutdown code called {0}")]
    Shutdown(ExitCode),
    #[error("{0}: command not found")]
    CommandNotFound(Cow<'name, str>),
    #[error("The command {0} is missing an argument")]
    MissingArgument(Cow<'name, str>),
    #[error("The incorrect argument {0} should be a {1}")]
    IncorrectArgumentType(Cow<'name, str>, Cow<'name, str>),
    #[error("Path is not valid {0}")]
    IncorrectArgument(Cow<'name, str>),
    #[error("Io Error {0}")]
    IoError(#[from] std::io::Error),
}

enum RedirectIO<T: std::io::Write + Into<Stdio>> {
    File(File),
    Other(T),
}

impl<T> From<RedirectIO<T>> for Stdio
where
    T: std::io::Write + Into<Stdio>,
{
    fn from(value: RedirectIO<T>) -> Self {
        match value {
            RedirectIO::File(file) => file.into(),
            RedirectIO::Other(other) => other.into(),
        }
    }
}

impl<T> std::io::Write for RedirectIO<T>
where
    T: std::io::Write + Into<Stdio>,
{
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            RedirectIO::File(file) => file.write(buf),
            RedirectIO::Other(other) => other.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            RedirectIO::File(file) => file.flush(),
            RedirectIO::Other(other) => other.flush(),
        }
    }
}

struct Redirect {
    stdout: RedirectIO<Stdout>,
    stderr: RedirectIO<Stderr>,
}

impl Default for Redirect {
    fn default() -> Self {
        Self {
            stdout: RedirectIO::Other(std::io::stdout()),
            stderr: RedirectIO::Other(std::io::stderr()),
        }
    }
}

impl Redirect {
    fn new(args: &[Cow<'_, str>]) -> std::io::Result<(Self, Option<usize>)> {
        for (i, (operator, file_path)) in args.iter().tuple_windows().enumerate() {
            match operator.as_ref() {
                "1>" | ">" => {
                    let stdout = File::options()
                        .create(true)
                        .truncate(true)
                        .write(true)
                        .open(file_path.as_ref())?;

                    let s = Self {
                        stdout: RedirectIO::File(stdout),
                        ..Default::default()
                    };

                    return Ok((s, Some(i)));
                }
                "2>" => {
                    let stderr = File::options()
                        .create(true)
                        .truncate(true)
                        .write(true)
                        .open(file_path.as_ref())?;

                    let s = Self {
                        stderr: RedirectIO::File(stderr),
                        ..Default::default()
                    };

                    return Ok((s, Some(i)));
                }
                "1>>" | ">>" => {
                    let stdout = File::options()
                        .create(true)
                        .append(true)
                        .open(file_path.as_ref())?;

                    let s = Self {
                        stdout: RedirectIO::File(stdout),
                        ..Default::default()
                    };

                    return Ok((s, Some(i)));
                }
                "2>>" => {
                    let stderr = File::options()
                        .create(true)
                        .append(true)
                        .open(file_path.as_ref())?;

                    let s = Self {
                        stderr: RedirectIO::File(stderr),
                        ..Default::default()
                    };

                    return Ok((s, Some(i)));
                }
                _ => {}
            }
        }

        let stdout = RedirectIO::Other(std::io::stdout());
        let stderr = RedirectIO::Other(std::io::stderr());

        Ok((Self { stdout, stderr }, None))
    }
}

#[derive(Debug, Clone)]
enum Builtins {
    Exit,
    Echo,
    Type,
    Pwd,
    Cd,
    History,
}

impl<'input> TryFrom<&'input str> for Builtins {
    type Error = Errors<'input>;

    fn try_from(value: &'input str) -> Result<Self, Self::Error> {
        match value {
            "exit" => Ok(Self::Exit),
            "echo" => Ok(Self::Echo),
            "type" => Ok(Self::Type),
            "pwd" => Ok(Self::Pwd),
            "cd" => Ok(Self::Cd),
            "history" => Ok(Self::History),
            _ => Err(Errors::CommandNotFound(value.into())),
        }
    }
}

impl Builtins {
    fn supported() -> [&'static str; 6] {
        ["exit", "echo", "type", "pwd", "cd", "history"]
    }
}

struct State {
    last_exit_code: ExitCode,
    path: PathBuf,
    history: Vec<String>,
}

impl State {
    fn is_builtin(com: &str) -> Result<(), Errors<'_>> {
        com.try_into().map(|_: Builtins| ())
    }

    fn run_builtins<'name>(
        &mut self,
        com: Builtins,
        rest: &[Cow<'name, str>],
        stdout: &mut dyn std::io::Write,
        _stderr: &mut dyn std::io::Write,
    ) -> Result<(), Errors<'name>> {
        match com {
            Builtins::Exit => self.run_exit(rest),
            Builtins::Echo => self.run_echo(rest, stdout),
            Builtins::Type => self.run_type(rest, stdout),
            Builtins::History => self.run_history(rest, stdout),
            Builtins::Pwd => self.run_pwd(stdout),
            Builtins::Cd => self.run_cd(rest, stdout),
        }
    }

    fn run_cd<'name>(
        &mut self,
        rest: &[Cow<'name, str>],
        stdout: &mut dyn std::io::Write,
    ) -> Result<(), Errors<'name>> {
        let mut old = self.path.clone();
        let new = rest[0].clone();
        // absolute
        let new = if new.starts_with('/') {
            PathBuf::from_str(new.as_ref()).or(Err(Errors::IncorrectArgument(new)))?
        } else if new.starts_with('~') {
            // home case
            let hm = std::env::var("HOME").expect("error getting HOME env variable");
            let mut hm = PathBuf::from_str(&hm).expect("HOME Environment variable is not valid");
            hm.push(new.trim_start_matches('~'));
            hm
        } else {
            old.push(new.as_ref());
            old
        };

        if new.is_dir() {
            self.path = std::fs::canonicalize(new)?;
        } else {
            let p = format!("{:?}", new);
            writeln!(
                stdout,
                "cd: {}: No such file or directory",
                p.trim_matches('"')
            )?;
        }
        Ok(())
    }

    fn run_pwd<'name>(&mut self, stdout: &mut dyn std::io::Write) -> Result<(), Errors<'name>> {
        let p = format!("{:?}", self.path);
        writeln!(stdout, "{}", p.trim_matches('"'))?;
        Ok(())
    }

    fn run_history<'name>(
        &mut self,
        rest: &[Cow<'name, str>],
        stdout: &mut dyn std::io::Write,
    ) -> Result<(), Errors<'name>> {
        let len = if rest.is_empty() {
            self.history.len()
        } else {
            match rest[0].parse() {
                Ok(k) => k,
                Err(_) => {
                    return Err(Errors::IncorrectArgumentType(
                        rest[0].clone(),
                        "integer".into(),
                    ));
                }
            }
        };
        for (i, l) in self
            .history
            .iter()
            .enumerate()
            .skip(self.history.len() - len)
        {
            writeln!(stdout, "    {} {}", i + 1, l)?;
        }
        Ok(())
    }

    fn run_type<'name>(
        &mut self,
        rest: &[Cow<'name, str>],
        stdout: &mut dyn std::io::Write,
    ) -> Result<(), Errors<'name>> {
        let com = rest[0].clone();
        if Self::is_builtin(com.as_ref()).is_ok() {
            writeln!(stdout, "{} is a shell builtin", com)?;
        } else if let Ok(v) = Self::is_program(&com) {
            writeln!(stdout, "{} is {}", com, v)?;
        } else {
            writeln!(stdout, "{} not found", com)?;
        }
        Ok(())
    }

    fn run_echo<'name>(
        &mut self,
        rest: &[Cow<'name, str>],
        stdout: &mut dyn std::io::Write,
    ) -> Result<(), Errors<'name>> {
        writeln!(stdout, "{}", rest.join(" "))?;
        Ok(())
    }

    fn run_exit<'name>(&mut self, rest: &[Cow<'name, str>]) -> Result<(), Errors<'name>> {
        if rest.is_empty() {
            return Err(Errors::MissingArgument("exit".into()));
        }

        let code = rest[0].parse();
        if let Ok(c) = code {
            return Err(Errors::Shutdown(c));
        }

        Err(Errors::IncorrectArgumentType(
            rest[0].clone(),
            "integer".into(),
        ))
    }

    fn is_program<'a>(com: &Cow<'a, str>) -> Result<String, Errors<'a>> {
        let paths = std::env::var("PATH").expect("PATH should have been set correctly");
        let mut pbuf = PathBuf::new();
        for path in paths.split(':').map(str::trim) {
            pbuf.clear();
            pbuf.push(path);
            pbuf.push(com.as_ref());
            if pbuf.is_file() {
                return Ok(pbuf
                    .to_str()
                    .expect("unable to create string because of invalid UTF8")
                    .to_string());
            }
        }
        Err(Errors::CommandNotFound(com.clone()))
    }

    fn run_program<'com>(
        &self,
        com: &Cow<'com, str>,
        rest: &[Cow<'com, str>],
        stdout: impl Into<Stdio>,
        stderr: impl Into<Stdio>,
    ) -> Result<(), Errors<'com>> {
        let _path = Self::is_program(com)?;

        // ugly alloc
        let args: Vec<_> = rest.iter().map(AsRef::as_ref).collect();
        let mut child = Command::new(com.as_ref())
            .args(args)
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
            .expect("Failed to execute the child process");

        let code = child.wait().expect("Failed to wait on child");
        let code = code.code().unwrap_or(0);

        if code == 0 {
            Ok(())
        } else {
            Err(Errors::ExitCode(code))
        }
    }

    fn run_commands<'com>(&mut self, command: &'com str) -> Result<(), Errors<'com>> {
        let args = args::process_args(command);

        let (mut redirect, from) = Redirect::new(&args)?;

        // make sure that the redirect information is not passed to processing
        let args = match from {
            Some(s) => &args[..s],
            None => &args[..],
        };

        if let Ok(com) = args[0].as_ref().try_into() {
            return self.run_builtins(com, &args[1..], &mut redirect.stdout, &mut redirect.stderr);
        }

        match self.run_program(&args[0], &args[1..], redirect.stdout, redirect.stderr) {
            Ok(_) => Ok(()),
            Err(Errors::CommandNotFound(_)) => Err(Errors::CommandNotFound(args[0].clone())),
            err @ Err(_) => err,
        }
    }
}

type Completion = trie_rs::Trie<u8>;

#[derive(Debug, thiserror::Error)]
enum TabHandlingError {
    #[error("PATH env is not set")]
    MissingPathEnv,
    #[error("File in PATH has not existing path name <{0}>")]
    FileNameMissing(PathBuf),
    #[error("Io Error <{0}>")]
    Io(#[from] std::io::Error),
}

fn generate_program_names() -> Result<Vec<String>, TabHandlingError> {
    let mut v = vec![];
    let paths = std::env::var("PATH").map_err(|_| TabHandlingError::MissingPathEnv)?;

    let process_file = |file: PathBuf| {
        file.file_name()
            .and_then(OsStr::to_str)
            .map(str::to_owned)
            .ok_or(TabHandlingError::FileNameMissing(file))
    };

    for path in std::env::split_paths(&paths).filter(|e| e.exists()) {
        for file in std::fs::read_dir(path)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|e| e.is_file())
        {
            let name = process_file(file)?;
            v.push(name);
        }
    }

    Ok(v)
}

fn generate_completion() -> Result<Completion, TabHandlingError> {
    let mut builder = trie_rs::TrieBuilder::new();

    // add builtins
    for s in Builtins::supported() {
        builder.push(s);
    }

    // program names
    let exes = generate_program_names()?;
    for s in exes {
        builder.push(s);
    }

    Ok(builder.build())
}

#[derive(Clone, Copy)]
enum TabCompletionState {
    /// No completion required
    None,
    FirstRound,
    // SecondRound,
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

            return Ok(TabCompletionState::FirstRound);
        }
    }

    if let TabCompletionState::None = state {
        // ring the bell
        stdout.execute(style::Print(BELL))?;
        return Ok(TabCompletionState::FirstRound);
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

const PROMT: &str = "$ ";
const BELL: char = '\u{07}';
const NEWLINE_RAW_TERM: &str = "\r\n";

#[derive(Debug, thiserror::Error)]
enum ReadLineError {
    #[error("Programshutdown")]
    Shutdown(i32),
    #[error("IO ERROR <{0}>")]
    Io(#[from] std::io::Error),
    #[error("While handling the tab completion <{0}>")]
    TabHandling(#[from] TabHandlingError),
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
    let completion = generate_completion()?;
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

fn read_line(
    line: &mut String,
    stdout: &mut Stdout,
    history: &[String],
) -> Result<(), ReadLineError> {
    enable_raw_mode()?;
    let res = read_line_loop(line, stdout, history);
    disable_raw_mode()?;
    res
}

pub fn repl() -> anyhow::Result<Option<ExitCode>> {
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();

    let mut input = String::with_capacity(1024);

    let history = Vec::with_capacity(100);

    let mut state = State {
        last_exit_code: 0,
        path: std::env::current_dir().context("Current directory is invalid?")?,
        history,
    };

    let mut shutdown_code = None;

    loop {
        input.clear();

        // add promt
        write!(&stdout, "{}", PROMT)?;

        stdout.flush()?;
        stderr.flush()?;

        match read_line(&mut input, &mut stdout, &state.history) {
            Ok(_) => {}
            Err(ReadLineError::Shutdown(0)) => {
                break;
            }
            Err(ReadLineError::Shutdown(v)) => {
                shutdown_code = Some(v);
                break;
            }
            Err(err) => {
                writeln!(&stdout, "Error: {:?}", err)?;
                shutdown_code = Some(1);
                break;
            }
        }

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        state.history.push(input.to_string());

        match state.run_commands(input) {
            Ok(_) => state.last_exit_code = 0,
            Err(Errors::CommandNotFound(_)) => {
                writeln!(&stdout, "{}: command not found", input)?;
            }
            Err(Errors::ExitCode(v)) => {
                state.last_exit_code = v;
            }
            Err(Errors::Shutdown(v)) => {
                shutdown_code = Some(v);
                break;
            }
            Err(e @ Errors::MissingArgument(_)) => {
                writeln!(&stdout, "{}", e)?;
            }
            Err(e @ Errors::IncorrectArgumentType(_, _)) => {
                writeln!(&stdout, "{}", e)?;
            }
            Err(e @ Errors::IncorrectArgument(_)) => {
                writeln!(&stdout, "{}", e)?;
            }
            Err(e @ Errors::IoError(_)) => {
                writeln!(&stdout, "{}", e)?;
            }
        }
    }

    Ok(shutdown_code)
}
