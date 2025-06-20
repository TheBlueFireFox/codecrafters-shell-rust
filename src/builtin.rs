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

pub mod history {
    use std::{
        borrow::Cow,
        io::{Read, Seek, Write},
        path::{Path, PathBuf},
    };

    use itertools::Itertools;

    use crate::repl::State;

    use super::Errors;

    pub fn run(
        state: &mut State,
        rest: &[Cow<'_, str>],
        stdout: &mut dyn std::io::Write,
    ) -> Result<(), Errors> {
        match rest.first().map(AsRef::as_ref) {
            None => state.history.print_history(None, stdout),
            Some("-r") => state.history.read_history(&rest[1..]),
            Some("-w") => state.history.write_history(&rest[1..]),
            Some("-a") => state.history.append_history(&rest[1..]),
            Some(v) => {
                let count = v.parse().map_err(|_| {
                    Errors::IncorrectArgumentType(v.to_string(), "Integer".to_string())
                })?;
                state.history.print_history(Some(count), stdout)
            }
        }
    }

    #[derive(Clone, Debug)]
    pub struct History {
        pub history: Vec<String>,
        appended: usize,
        org: Option<OrgFile>,
    }

    #[derive(Clone, Debug)]
    struct OrgFile {
        path: PathBuf,
        read_lines: usize,
    }

    impl Drop for History {
        fn drop(&mut self) {
            let Some(org) = self.org.clone() else {
                return;
            };
            Self::append_history_to_file(self, org.path, org.read_lines)
                .expect("can write to history");
        }
    }

    impl History {
        pub fn new() -> Result<Self, Errors> {
            let s = Self {
                history: Vec::with_capacity(100),
                appended: 0,
                org: None,
            };
            Ok(s)
        }
        pub fn from_file(path: impl AsRef<Path>) -> Result<Self, Errors> {
            let lines = Self::read_history_from_file(path.as_ref())?;
            let len = lines.len();
            let s = Self {
                history: lines,
                appended: 0,
                org: Some(OrgFile {
                    path: path.as_ref().into(),
                    read_lines: len,
                }),
            };

            Ok(s)
        }

        fn write_history(&mut self, rest: &[Cow<'_, str>]) -> Result<(), Errors> {
            match rest.first() {
                None => Err(Errors::MissingArgument("history".to_string())),
                Some(path) => {
                    let mut file = std::fs::OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(path.as_ref())?;

                    let s = self.history.join("\n");
                    file.write_all(s.as_bytes())?;

                    self.appended = self.history.len();

                    Ok(())
                }
            }
        }

        fn append_history(&mut self, rest: &[Cow<'_, str>]) -> Result<(), Errors> {
            match rest.first() {
                None => Err(Errors::MissingArgument("history".to_string())),
                Some(path) => Self::append_history_to_file(self, path.as_ref(), self.appended),
            }
        }

        fn read_history(&mut self, rest: &[Cow<'_, str>]) -> Result<(), Errors> {
            match rest.first() {
                None => Err(Errors::MissingArgument("history".to_string())),
                Some(path) => {
                    let lines = Self::read_history_from_file(path.as_ref())?;
                    self.history.extend(lines);
                    self.appended = self.history.len();
                    Ok(())
                }
            }
        }

        fn print_history(
            &self,
            count: Option<usize>,
            stdout: &mut dyn std::io::Write,
        ) -> Result<(), Errors> {
            let other = count.unwrap_or(self.history.len());
            let s = self.history.len().saturating_sub(other);

            for (i, l) in self.history.iter().enumerate().skip(s) {
                writeln!(stdout, "    {} {}", i + 1, l)?;
            }
            Ok(())
        }

        fn read_history_from_file(path: impl AsRef<Path>) -> Result<Vec<String>, Errors> {
            let mut file = std::fs::OpenOptions::new().read(true).open(path.as_ref())?;
            let mut s = String::new();
            file.read_to_string(&mut s)?;
            let s = s.lines().map(String::from).collect();
            Ok(s)
        }

        fn append_history_to_file(
            &mut self,
            path: impl AsRef<Path>,
            count: usize,
        ) -> Result<(), Errors> {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(path.as_ref())?;

            let it =
                self.history[count..]
                    .iter()
                    .map(AsRef::as_ref)
                    .interleave(itertools::repeat_n(
                        "\n",
                        self.history.len().saturating_sub(count),
                    ));

            let mut s = String::new();
            file.read_to_string(&mut s)?;

            if !s.ends_with('\n') {
                s.push('\n');
            }

            file.set_len(0)?;

            file.seek(std::io::SeekFrom::Start(0))?;

            s.extend(it);
            file.write_all(s.as_bytes())?;

            self.appended = self.history.len();
            Ok(())
        }
    }
}
