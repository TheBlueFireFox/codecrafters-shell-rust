#![allow(dead_code)]

use std::{
    borrow::Cow,
    io::{self, Write},
    path::PathBuf,
    process::Command,
    str::FromStr,
};

use itertools::Itertools;

fn main() {
    repl();
}

type ExitCode = i32;

#[derive(thiserror::Error, Debug)]
enum Errors<'name> {
    #[error("exit code called {0}")]
    ExitCode(ExitCode),
    #[error("{0}: command not found")]
    CommandNotFound(Cow<'name, str>),
    #[error("The command {0} is missing an argument")]
    MissingArgument(Cow<'name, str>),
    #[error("The incorrect argument {0} should be a {1}")]
    IncorrectArgumentType(Cow<'name, str>, Cow<'name, str>),
    #[error("Path is not valid {0}")]
    IncorrectArgument(Cow<'name, str>),
}

enum Builtins {
    Exit,
    Echo,
    Type,
    Pwd,
    Cd,
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
            _ => Err(Errors::CommandNotFound(value.into())),
        }
    }
}

struct State {
    last_exit_code: ExitCode,
    path: PathBuf,
}

impl State {
    fn is_builtin(com: &str) -> Result<(), Errors<'_>> {
        com.try_into().map(|_: Builtins| ())
    }

    fn run_builtins<'name>(
        &mut self,
        com: Builtins,
        rest: &[Cow<'name, str>],
    ) -> Result<(), Errors<'name>> {
        match com {
            Builtins::Exit => {
                if rest.is_empty() {
                    return Err(Errors::MissingArgument("exit".into()));
                }

                let code = rest[0].parse();
                if let Ok(c) = code {
                    std::process::exit(c);
                }
                Err(Errors::IncorrectArgumentType(
                    rest[0].clone(),
                    "integer".into(),
                ))
            }
            Builtins::Echo => {
                println!("{}", rest.join(" "));
                io::stdout().flush().unwrap();
                Ok(())
            }
            Builtins::Type => {
                let com = rest[0].clone();
                if Self::is_builtin(com.as_ref()).is_ok() {
                    println!("{} is a shell builtin", com);
                } else if let Ok(v) = Self::is_program(com.as_ref()) {
                    println!("{} is {}", com, v);
                } else {
                    println!("{} not found", com)
                }
                io::stdout().flush().unwrap();
                Ok(())
            }
            Builtins::Pwd => {
                let p = format!("{:?}", self.path);
                println!("{}", p.trim_matches('"'));
                io::stdout().flush().unwrap();
                Ok(())
            }
            Builtins::Cd => {
                let mut old = self.path.clone();
                let new = rest[0].clone();
                // absolute
                let new = if new.starts_with('/') {
                    PathBuf::from_str(new.as_ref()).or(Err(Errors::IncorrectArgument(new)))?
                } else if new.starts_with('~') {
                    // home case
                    let hm = std::env::var("HOME").expect("error getting HOME env variable");
                    let mut hm =
                        PathBuf::from_str(&hm).expect("HOME Environment variable is not valid");
                    hm.push(new.trim_start_matches('~'));
                    hm
                } else {
                    old.push(new.as_ref());
                    old
                };

                if new.is_dir() {
                    self.path = std::fs::canonicalize(new).expect("Path should exists");
                } else {
                    let p = format!("{:?}", new);
                    println!("cd: {}: No such file or directory", p.trim_matches('"'));
                    io::stdout().flush().unwrap();
                }
                Ok(())
            }
        }
    }

    fn is_program(com: &str) -> Result<String, Errors<'_>> {
        let paths = std::env::var("PATH").expect("PATH should have been set correctly");
        let mut pbuf = PathBuf::new();
        for path in paths.split(':').map(str::trim) {
            pbuf.clear();
            pbuf.push(path);
            pbuf.push(com);
            if pbuf.is_file() {
                return Ok(pbuf
                    .to_str()
                    .expect("unable to create string because of invalid UTF8")
                    .to_string());
            }
        }
        Err(Errors::CommandNotFound(com.into()))
    }

    fn run_program<'com>(
        &self,
        com: &'com str,
        rest: &[Cow<'com, str>],
    ) -> Result<(), Errors<'com>> {
        let _path = Self::is_program(com)?;

        // ugly alloc
        let args: Vec<_> = rest.iter().map(AsRef::as_ref).collect();
        let mut child = Command::new(com)
            .args(args)
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
        let (com, rest) = command.split_once(' ').unwrap_or((command, ""));
        let parts: Vec<_> = process_args(rest);

        if let Ok(com) = com.try_into() {
            return self.run_builtins(com, &parts);
        }

        match self.run_program(com, &parts) {
            Ok(_) => Ok(()),
            Err(Errors::CommandNotFound(_)) => Err(Errors::CommandNotFound(com.into())),
            err @ Err(_) => err,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Character {
    SingleQuote,
    DoubleQuote,
    WhiteSpace,
    Other,
}

impl Character {
    fn map(c: char) -> Self {
        match c {
            '\'' => Self::SingleQuote,
            '"' => Self::DoubleQuote,
            ' ' => Self::WhiteSpace,
            _ => Self::Other,
        }
    }
}

fn process_args(args_raw: &str) -> Vec<Cow<'_, str>> {
    let mut v: Vec<Cow<'_, str>> = vec![];

    let mut current_block = Character::WhiteSpace;
    let mut last_idx = 0;

    let mut it = args_raw.chars().chain([' ']).tuple_windows().enumerate();

    while let Some((idx, (c1, c2))) = it.next() {
        match (c1, c2) {
            ('\\', '\\') => {
                it.next();
                it.next();
                continue;
            }
            ('\\', '\"') => {
                it.next();
                it.next();
                continue;
            }
            ('\\', x) => {
                unimplemented!("not supported escape \\{x}");
            }
            _ => {}
        }
        match (current_block, Character::map(c1)) {
            (Character::SingleQuote, Character::SingleQuote) => {
                // case 'XX' <-
                // finished text block
                // + 1 to ignore '
                // ..idx to ignore '
                if !c2.is_whitespace() {
                    it.next();
                    continue;
                }

                let s = &args_raw[last_idx..=idx];
                v.push(s.into());
                current_block = Character::WhiteSpace;
            }
            (Character::SingleQuote, Character::DoubleQuote) => {
                // case: '" <-
            }
            (Character::SingleQuote, Character::WhiteSpace) => {
                // case: '_ <-
            }
            (Character::SingleQuote, Character::Other) => {
                // case: 'X <-
            }
            (Character::DoubleQuote, Character::SingleQuote) => {
                // case: "' <-
            }
            (Character::DoubleQuote, Character::DoubleQuote) => {
                // case: "XX" <-
                // finished text block
                // + 1 to ignore '
                // ..idx to ignore '
                if !c2.is_whitespace() {
                    it.next();
                    continue;
                }
                let s = &args_raw[last_idx..=idx];
                v.push(s.into());
                current_block = Character::WhiteSpace;
            }
            (Character::DoubleQuote, Character::WhiteSpace) => {}
            (Character::DoubleQuote, Character::Other) => {}
            (Character::WhiteSpace, Character::SingleQuote) => {
                // case: _' <-
                current_block = Character::SingleQuote;
                last_idx = idx;
            }
            (Character::WhiteSpace, Character::DoubleQuote) => {
                current_block = Character::DoubleQuote;
                last_idx = idx;
            }
            (Character::WhiteSpace, Character::WhiteSpace) => {}
            (Character::WhiteSpace, Character::Other) => {
                // case: _X <-
                current_block = Character::Other;
                last_idx = idx;
            }
            (Character::Other, Character::SingleQuote) => {
                // case: X' <-
                // b'example'a => bexamplea

                // let s = &args_raw[last_idx..=idx];
                // v.push(s);

                // last_idx = idx;
                current_block = Character::SingleQuote;
            }
            (Character::Other, Character::DoubleQuote) => {
                // case: XX" <-

                // let s = &args_raw[last_idx..=idx];
                // v.push(s.into());

                // last_idx = idx;
                current_block = Character::DoubleQuote;
            }
            (Character::Other, Character::WhiteSpace) => {
                let s = &args_raw[last_idx..idx];
                v.push(s.into());

                current_block = Character::WhiteSpace;
            }
            (Character::Other, Character::Other) => {
                // case: XX <-
            }
        }
    }

    match current_block {
        Character::SingleQuote => {
            unimplemented!("missing end quote")
        }
        Character::DoubleQuote => {
            unimplemented!("missing end double quote")
        }
        Character::WhiteSpace => {}
        Character::Other => {
            let s = &args_raw[last_idx..];
            v.push(s.into());
        }
    }

    for arg in v.iter_mut() {
        // trying to process the argument in a single pass
        let mut it = arg.chars().tuple_windows().enumerate().peekable();

        let mut processing_required = None;

        // check if any of these blocks need processing
        while let Some((idx, (c1, c2))) = it.peek().copied() {
            match (c1, c2) {
                ('\'', _) | ('"', _) | ('\\', _) => {
                    // a ' => needs processing
                    // a " => needs processing
                    // escaped symbol => needs processing
                    processing_required = Some(idx);
                    break;
                }
                _ => {
                    // we don't care about this combination
                }
            }
            // consume the token
            it.next();
        }

        let mut s = match processing_required {
            None => continue,
            Some(c) => {
                let mut s = String::with_capacity(arg.len());
                // add the clean blocks
                s.push_str(&arg[..c]);
                s
            }
        };

        let mut last_char = ' ';
        let mut current_context = Character::WhiteSpace;

        while let Some((_, (c1, c2))) = it.next() {
            last_char = c2;
            match (c1, c2) {
                ('\'', _) => {
                    // a ' => needs processing
                    // a " => needs processing
                    // escaped symbol => needs processing
                    match current_context {
                        Character::SingleQuote => {
                            current_context = Character::WhiteSpace;
                        }
                        Character::DoubleQuote => {
                            s.push('\'');
                        }
                        Character::WhiteSpace => {
                            current_context = Character::SingleQuote;
                        }
                        Character::Other => {
                            unimplemented!("If I get this one I messed up");
                        }
                    }
                }
                ('"', _) => match current_context {
                    Character::SingleQuote => {
                        s.push('"');
                    }
                    Character::DoubleQuote => current_context = Character::WhiteSpace,
                    Character::WhiteSpace => current_context = Character::DoubleQuote,
                    Character::Other => {}
                },
                ('\\', '\\') => {
                    s.push('\\');
                    it.next();
                    it.next();
                }
                ('\\', '\"') => {
                    s.push('\"');
                    it.next();
                    it.next();
                }
                ('\\', x) => {
                    unimplemented!("not supported escape \\{x}");
                }
                v => {
                    // we don't care about this combination
                    s.push(v.0);
                }
            }
        }

        if !matches!(last_char, '\'' | '"' | '\\') {
            s.push(last_char);
        }

        *arg = Cow::Owned(s);
    }

    v
}

fn repl() {
    let stdin = io::stdin();
    let mut input = String::new();

    let mut state = State {
        last_exit_code: 0,
        path: std::env::current_dir().expect("Current directory is invalid?"),
    };

    loop {
        input.clear();

        // add promt
        print!("$ ");
        io::stdout().flush().unwrap();
        let size = stdin.read_line(&mut input).unwrap();
        if size == 0 {
            println!();
            break;
        }
        let input = input.trim();
        if input.is_empty() {
            continue;
        }
        match state.run_commands(input) {
            Ok(_) => state.last_exit_code = 0,
            Err(Errors::CommandNotFound(_)) => {
                println!("{}: command not found", input);
                io::stdout().flush().unwrap();
            }
            Err(Errors::ExitCode(v)) => {
                state.last_exit_code = v;
            }
            Err(e @ Errors::MissingArgument(_)) => {
                println!("{}", e);
                io::stdout().flush().unwrap();
            }
            Err(e @ Errors::IncorrectArgumentType(_, _)) => {
                println!("{}", e);
                io::stdout().flush().unwrap();
            }
            Err(e @ Errors::IncorrectArgument(_)) => {
                println!("{}", e);
                io::stdout().flush().unwrap();
            }
        }

        // read input
        // process
        // output processed
    }
}

#[cfg(test)]
mod test {
    use std::borrow::Cow;

    use crate::process_args;

    #[test]
    fn test_process_args_simple() {
        let txt = "foo";
        let exp: &[Cow<'_, str>] = &["foo"].map(Into::into);
        let v = process_args(txt);
        assert_eq!(exp, &v[..]);
    }

    #[test]
    fn test_process_args_simple_multiple() {
        let txt = "foo XX";
        let exp: &[Cow<'_, str>] = &["foo", "XX"].map(Into::into);

        let v = process_args(txt);
        assert_eq!(exp, &v[..]);
    }

    #[test]
    fn test_process_args_single_single_quote() {
        let txt = "'XX'";
        let exp: &[Cow<'_, str>] = &["XX"].map(Into::into);
        let v = process_args(txt);
        assert_eq!(exp, &v[..]);
    }

    #[test]
    fn test_process_args_multiple_single_quote() {
        let txt = "'AA    ''BB' 'CC'";
        let exp: &[Cow<'_, str>] = &["AA    BB", "CC"].map(Into::into);
        let v = process_args(txt);
        assert_eq!(exp, &v[..]);
    }

    #[test]
    fn test_process_args_single_quote_with_double() {
        let txt = "'\"AA\"'";
        let exp: &[Cow<'_, str>] = &["\"AA\""].map(Into::into);
        let v = process_args(txt);
        assert_eq!(exp, &v[..]);
    }

    #[test]
    fn test_process_args_double_quote() {
        let txt = "\"XX\"";
        let exp: &[Cow<'_, str>] = &["XX"].map(Into::into);
        let v = process_args(txt);
        assert_eq!(exp, &v[..]);
    }

    #[test]
    fn test_process_args_multiple_double_quote() {
        let txt = "\"AA\"\"BB\" \"CC\"";
        let exp: &[Cow<'_, str>] = &["AABB", "CC"].map(Into::into);
        let v = process_args(txt);
        assert_eq!(exp, &v[..]);
    }

    #[test]
    fn test_process_args_double_quote_with_single() {
        let txt = "\"'AA'\"";
        let exp: &[Cow<'_, str>] = &["'AA'"].map(Into::into);
        let v = process_args(txt);
        assert_eq!(exp, &v[..]);
    }
}
