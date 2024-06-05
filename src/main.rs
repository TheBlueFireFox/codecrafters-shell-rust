#![allow(dead_code)]

use std::io::{self, Write};

fn main() {
    repl();
}

type ExitCode = i32;

struct State {
    last_exit_code: ExitCode,
}

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
}

enum Builtins {
    Exit,
    Echo,
    Type,
}

impl<'input> TryFrom<&'input str> for Builtins {
    type Error = Errors<'input>;

    fn try_from(value: &'input str) -> Result<Self, Self::Error> {
        match value {
            "exit" => Ok(Self::Exit),
            "echo" => Ok(Self::Echo),
            "type" => Ok(Self::Type),
            _ => Err(Errors::CommandNotFound(value)),
        }
    }
}

fn is_builtin(com: &str) -> Result<(), Errors<'_>> {
    com.try_into().map(|_: Builtins| ())
}

fn run_builtins<'name>(com: Builtins, rest: &[&'name str]) -> Result<(), Errors<'name>> {
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
            if let Ok(v) = is_program(com) {
                println!("{} is {}", com, v);
            } else if is_builtin(com).is_ok() {
                println!("{} is a shell builtin", com);
            } else {
                println!("{} not found", com)
            }
            io::stdout().flush().unwrap();
            Ok(())
        }
    }
}

fn is_program(com: &str) -> Result<String, Errors<'_>> {
    Err(Errors::CommandNotFound(com))
}

fn run_program<'name>(com: &'name str, _rest: &[&'name str]) -> Result<(), Errors<'name>> {
    Err(Errors::CommandNotFound(com))
}

fn run_commands(command: &str) -> Result<(), Errors> {
    let (com, rest) = command.split_once(' ').unwrap_or((command, ""));
    let parts: Vec<_> = rest.split_whitespace().collect();
    // TODO: match commands
    match run_program(com, &parts) {
        Ok(_) => return Ok(()),
        Err(Errors::CommandNotFound(_)) => {}
        err @ Err(_) => return err,
    }
    match com.try_into() {
        Ok(com) => run_builtins(com, &parts),
        Err(_) => Err(Errors::CommandNotFound(com)),
    }
}

fn repl() {
    let stdin = io::stdin();
    let mut input = String::new();

    let mut state = State { last_exit_code: 0 };
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
        match run_commands(input) {
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
        }

        // read input
        // process
        // output processed
    }
}
