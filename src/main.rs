#![allow(dead_code)]

use std::{
    io::{self, Write},
    path::PathBuf,
    process::Command,
    str::FromStr,
};

fn main() {
    repl();
}

type ExitCode = i32;

#[derive(thiserror::Error, Debug)]
enum Errors<'name> {
    #[error("exit code called {0}")]
    ExitCode(ExitCode),
    #[error("{0}: command not found")]
    CommandNotFound(&'name str),
    #[error("The command {0} is missing an argument")]
    MissingArgument(&'name str),
    #[error("The incorrect argument {0} should be a {1}")]
    IncorrectArgumentType(&'name str, &'name str),
    #[error("Path is not valid {0}")]
    IncorrectArgument(&'name str),
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
            _ => Err(Errors::CommandNotFound(value)),
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
        rest: &[&'name str],
    ) -> Result<(), Errors<'name>> {
        match com {
            Builtins::Exit => {
                if rest.is_empty() {
                    return Err(Errors::MissingArgument("exit"));
                }

                let code = rest[0].parse();
                if let Ok(c) = code {
                    std::process::exit(c);
                }
                Err(Errors::IncorrectArgumentType(rest[0], "integer"))
            }
            Builtins::Echo => {
                println!("{}", rest.join(" "));
                io::stdout().flush().unwrap();
                Ok(())
            }
            Builtins::Type => {
                let com = rest[0];
                if Self::is_builtin(com).is_ok() {
                    println!("{} is a shell builtin", com);
                } else if let Ok(v) = Self::is_program(com) {
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
                let new = rest[0];
                // absolute
                let new = if new.starts_with('/') {
                    PathBuf::from_str(new).or(Err(Errors::IncorrectArgument(new)))?
                } else if new.starts_with('~') {
                    // home case
                    let hm = std::env::var("HOME").expect("error getting HOME env variable");
                    let mut hm =
                        PathBuf::from_str(&hm).expect("HOME Environment variable is not valid");
                    hm.push(new.trim_start_matches('~'));
                    hm
                } else {
                    old.push(new);
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
        Err(Errors::CommandNotFound(com))
    }

    fn run_program<'com>(&self, com: &'com str, rest: &[&'com str]) -> Result<(), Errors<'com>> {
        match Self::is_program(com) {
            Err(_) => Err(Errors::CommandNotFound(com)),
            Ok(path) => {
                let mut child = Command::new(path)
                    .args(rest)
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
        }
    }

    fn run_commands<'com>(&mut self, command: &'com str) -> Result<(), Errors<'com>> {
        let (com, rest) = command.split_once(' ').unwrap_or((command, ""));
        let parts: Vec<_> = rest.split_whitespace().collect();

        if let Ok(com) = com.try_into() {
            return self.run_builtins(com, &parts);
        }

        match self.run_program(com, &parts) {
            Ok(_) => Ok(()),
            Err(Errors::CommandNotFound(_)) => Err(Errors::CommandNotFound(com)),
            err @ Err(_) => err,
        }
    }
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
