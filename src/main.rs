mod args;

use std::{
    borrow::Cow,
    io::{self, Write},
    path::PathBuf,
    process::Command,
    str::FromStr,
};

use anyhow::Context as _;

fn main() -> anyhow::Result<()> {
    repl()
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
    #[error("Io Error {0}")]
    IoError(#[from] std::io::Error),
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

                return Err(Errors::IncorrectArgumentType(
                    rest[0].clone(),
                    "integer".into(),
                ));
            }
            Builtins::Echo => {
                println!("{}", rest.join(" "));
            }
            Builtins::Type => {
                let com = rest[0].clone();
                if Self::is_builtin(com.as_ref()).is_ok() {
                    println!("{} is a shell builtin", com);
                } else if let Ok(v) = Self::is_program(&com) {
                    println!("{} is {}", com, v);
                } else {
                    println!("{} not found", com)
                }
            }
            Builtins::Pwd => {
                let p = format!("{:?}", self.path);
                println!("{}", p.trim_matches('"'));
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
                    self.path = std::fs::canonicalize(new)?;
                } else {
                    let p = format!("{:?}", new);
                    println!("cd: {}: No such file or directory", p.trim_matches('"'));
                }
            }
        }
        Ok(())
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
    ) -> Result<(), Errors<'com>> {
        let _path = Self::is_program(com)?;

        // ugly alloc
        let args: Vec<_> = rest.iter().map(AsRef::as_ref).collect();
        let mut child = Command::new(com.as_ref())
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

        let args= args::process_args(command);

        if let Ok(com) = args[0].as_ref().try_into() {
            return self.run_builtins(com, &args[1..]);
        }

        match self.run_program(&args[0], &args[1..]) {
            Ok(_) => Ok(()),
            Err(Errors::CommandNotFound(_)) => Err(Errors::CommandNotFound(args[0].clone())),
            err @ Err(_) => err,
        }
    }
}

fn repl() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut input = String::new();

    let mut state = State {
        last_exit_code: 0,
        path: std::env::current_dir().context("Current directory is invalid?")?,
    };

    loop {
        input.clear();

        // add promt
        print!("$ ");
        io::stdout().flush()?;

        let size = stdin.read_line(&mut input)?;
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
            }
            Err(Errors::ExitCode(v)) => {
                state.last_exit_code = v;
            }
            Err(e @ Errors::MissingArgument(_)) => {
                println!("{}", e);
            }
            Err(e @ Errors::IncorrectArgumentType(_, _)) => {
                println!("{}", e);
            }
            Err(e @ Errors::IncorrectArgument(_)) => {
                println!("{}", e);
            }
            Err(e @ Errors::IoError(_)) => {
                println!("{}", e);
            }
        }
    }

    Ok(())
}
