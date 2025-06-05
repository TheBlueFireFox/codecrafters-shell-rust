pub type ExitCode = i32;

use crate::args;
use std::{borrow::Cow, path::PathBuf};

use crate::repl::State;

#[derive(thiserror::Error, Debug)]
pub enum Errors {
    #[error("exit code called {0}")]
    ExitCode(ExitCode),
    #[error("shutdown code called {0}")]
    Shutdown(ExitCode),
    #[error("{0}: command not found")]
    CommandNotFound(String),
    #[error("The command {0} is missing an argument")]
    MissingArgument(String),
    #[error("The incorrect argument {0} should be a {1}")]
    IncorrectArgumentType(String, String),
    #[error("Io Error <{0}>")]
    IoError(#[from] std::io::Error),
    #[error("Parse Error {0}")]
    ParseError(#[from] args::Error),
}

#[derive(Debug, Clone)]
pub enum Builtins {
    Exit,
    Echo,
    Type,
    Pwd,
    Cd,
    History,
}

impl<'input> TryFrom<&'input str> for Builtins {
    type Error = Errors;

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
    pub fn supported() -> [&'static str; 6] {
        ["exit", "echo", "type", "pwd", "cd", "history"]
    }
}

pub fn is_program(com: impl AsRef<str>) -> Result<String, Errors> {
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
    Err(Errors::CommandNotFound(com.as_ref().to_string()))
}

pub fn run(
    state: &mut State,
    com: Builtins,
    rest: &[Cow<'_, str>],
    stdout: &mut dyn std::io::Write,
    _stderr: &mut dyn std::io::Write,
) -> Result<(), Errors> {
    match com {
        Builtins::Exit => exit::run(rest),
        Builtins::Echo => echo::run(rest, stdout),
        Builtins::Type => btype::run(rest, stdout),
        Builtins::Pwd => pwd::run(state, stdout),
        Builtins::Cd => cd::run(state, rest, stdout),
        Builtins::History => history::run(state, rest, stdout),
    }
}

mod cd {
    use std::{borrow::Cow, path::PathBuf};

    use crate::repl::State;

    use super::Errors;

    pub fn run(
        state: &mut State,
        rest: &[Cow<'_, str>],
        stdout: &mut dyn std::io::Write,
    ) -> Result<(), Errors> {
        let new = &rest[0];
        let new = match new.chars().next() {
            Some('~') => {
                // home case
                let hm: PathBuf = std::env::var("HOME")
                    .expect("error getting HOME env variable")
                    .into();
                hm.join(new.trim_start_matches('~'))
            }
            Some('/') => new.as_ref().into(),
            Some(_) => state.path.join(new.as_ref()),
            None => panic!("new path should not be empty"),
        };

        if new.is_dir() {
            state.path = std::fs::canonicalize(new)?;
            return Ok(());
        }

        let p = format!("{:?}", new);
        let p = p.trim_matches('"');
        writeln!(stdout, "cd: {}: No such file or directory", p)?;

        Ok(())
    }
}

mod btype {
    use std::borrow::Cow;

    use super::{is_program, Builtins, Errors};

    pub fn run(rest: &[Cow<'_, str>], stdout: &mut dyn std::io::Write) -> Result<(), Errors> {
        let com = rest[0].clone();
        if Builtins::try_from(com.as_ref()).is_ok() {
            writeln!(stdout, "{} is a shell builtin", com)?;
        } else if let Ok(v) = is_program(&com) {
            writeln!(stdout, "{} is {}", com, v)?;
        } else {
            writeln!(stdout, "{} not found", com)?;
        }
        Ok(())
    }
}

mod pwd {
    use crate::repl::State;

    use super::Errors;

    pub fn run(state: &State, stdout: &mut dyn std::io::Write) -> Result<(), Errors> {
        let p = format!("{:?}", state.path);
        writeln!(stdout, "{}", p.trim_matches('"'))?;
        Ok(())
    }
}

mod echo {
    use std::borrow::Cow;

    use super::Errors;

    pub fn run(rest: &[Cow<'_, str>], stdout: &mut dyn std::io::Write) -> Result<(), Errors> {
        writeln!(stdout, "{}", rest.join(" "))?;
        Ok(())
    }
}

mod exit {
    use std::borrow::Cow;

    use super::Errors;

    pub fn run(rest: &[Cow<'_, str>]) -> Result<(), Errors> {
        match rest.first() {
            None => Err(Errors::MissingArgument("exit".into())),
            Some(code) => {
                if let Ok(c) = code.parse() {
                    return Err(Errors::Shutdown(c));
                }
                Err(Errors::IncorrectArgumentType(
                    rest[0].to_string(),
                    "integer".into(),
                ))
            }
        }
    }
}

mod history {
    use std::{
        borrow::Cow,
        io::{Read, Write},
    };

    use crate::repl::State;

    use super::Errors;

    pub fn run(
        state: &mut State,
        rest: &[Cow<'_, str>],
        stdout: &mut dyn std::io::Write,
    ) -> Result<(), Errors> {
        match rest.first().map(AsRef::as_ref) {
            None => print_history(state, None, stdout),
            Some("-r") => read_history(state, &rest[1..]),
            Some("-w") => write_history(state, &rest[1..]),
            Some("-a") => append_history(state, &rest[1..]),
            Some(v) => {
                let count = v.parse().map_err(|_| {
                    Errors::IncorrectArgumentType(v.to_string(), "Integer".to_string())
                })?;
                print_history(state, Some(count), stdout)
            }
        }
    }

    fn write_history(state: &mut State, rest: &[Cow<'_, str>]) -> Result<(), Errors> {
        match rest.first() {
            None => Err(Errors::MissingArgument("history".to_string())),
            Some(path) => {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(path.as_ref())?;

                let mut s = state.history.history.join("\n");
                s.push('\n');
                file.write_all(s.as_bytes())?;

                state.history.appended = state.history.history.len();

                Ok(())
            }
        }
    }

    fn append_history(state: &mut State, rest: &[Cow<'_, str>]) -> Result<(), Errors> {
        match rest.first() {
            None => Err(Errors::MissingArgument("history".to_string())),
            Some(path) => {
                let mut file = std::fs::OpenOptions::new()
                    .append(true)
                    .read(true)
                    .create(true)
                    .open(path.as_ref())?;

                let mut s = state.history.history[state.history.appended..].join("\n");
                s.push('\n');
                file.write_all(s.as_bytes())?;

                state.history.appended = state.history.history.len();
                Ok(())
            }
        }
    }

    fn read_history(state: &mut State, rest: &[Cow<'_, str>]) -> Result<(), Errors> {
        match rest.first() {
            None => Err(Errors::MissingArgument("history".to_string())),
            Some(path) => {
                let mut file = std::fs::OpenOptions::new().read(true).open(path.as_ref())?;
                let mut s = String::new();
                file.read_to_string(&mut s)?;
                let lines = s.lines().filter(|s| !s.is_empty()).map(String::from);
                state.history.history.extend(lines);
                state.history.appended = state.history.history.len();
                Ok(())
            }
        }
    }

    fn print_history(
        state: &State,
        count: Option<usize>,
        stdout: &mut dyn std::io::Write,
    ) -> Result<(), Errors> {
        let other = count.unwrap_or(state.history.history.len());
        let s = state.history.history.len().saturating_sub(other);

        for (i, l) in state.history.history.iter().enumerate().skip(s) {
            writeln!(stdout, "    {} {}", i + 1, l)?;
        }
        Ok(())
    }
}
