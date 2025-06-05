pub type ExitCode = i32;

use crate::args;

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

// pub fn run() -> Result<(), Errors> {
//     todo!()
// }
