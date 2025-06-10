use std::{
    borrow::Cow,
    io::{Read, Seek, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
};

use anyhow::Context;

use memfile::MemFile;

use crate::{
    args,
    builtin::{self, history, Builtins, Errors, ExitCode},
    completion::{Completion, Entry, Type},
    redirect::{Redirect, RedirectIO},
    terminal::{read_line, ReadLineError, PROMT},
};

enum LastStdout {
    Child(Child),
    Builtin(RedirectIO<MemFile>),
    None,
}

impl From<LastStdout> for Stdio {
    fn from(value: LastStdout) -> Self {
        match value {
            LastStdout::Child(child) => child.stdout.map(Stdio::from).unwrap_or_else(Stdio::null),
            LastStdout::Builtin(file) => Stdio::from(file),
            LastStdout::None => Stdio::null(),
        }
    }
}

pub struct State {
    pub last_exit_code: ExitCode,
    pub path: PathBuf,
    pub history: history::History,
}

impl State {
    fn run_commands(&mut self, command: &str, completion: &Completion) -> Result<(), Errors> {
        let args = args::process_args(command)?;

        let mut last_stdout = LastStdout::None;
        let blocks = args.len();

        for (i, block) in args.into_iter().enumerate() {
            let is_last = i + 1 == blocks;

            last_stdout = match completion.matches_exact(&block.command) {
                None => Err(Errors::CommandNotFound(block.command.into())),
                Some(&Entry {
                    value: Type::Builtin(com),
                    ..
                }) => self.run_commands_builtin(completion, com, block, last_stdout),
                Some(&Entry {
                    value: Type::Program(_),
                    ..
                }) => self.run_commands_program(block, last_stdout, is_last),
            }?;
        }

        self.run_commands_post(last_stdout)
    }

    fn run_commands_post(&self, last_stdout: LastStdout) -> Result<(), Errors> {
        match last_stdout {
            LastStdout::None => Ok(()),
            LastStdout::Child(mut child) => {
                let code = child.wait()?;
                let code = code.code().unwrap_or(0);

                match code {
                    0 => Ok(()),
                    _ => Err(Errors::ExitCode(code)),
                }
            }

            LastStdout::Builtin(RedirectIO::Other(mut m)) => {
                let mut buf = String::new();
                m.read_to_string(&mut buf)?;
                print!("{}", buf);
                Ok(())
            }
            LastStdout::Builtin(RedirectIO::File(mut f)) => {
                let mut buf = String::new();
                f.read_to_string(&mut buf)?;
                print!("{}", buf);

                Ok(())
            }
        }
    }

    fn run_builtins(
        &mut self,
        completion: &Completion,
        com: Builtins,
        rest: &[Cow<'_, str>],
        stdout: &mut dyn std::io::Write,
        stderr: &mut dyn std::io::Write,
    ) -> Result<(), Errors> {
        builtin::run(self, completion, com, rest, stdout, stderr)
    }

    fn run_program(
        &self,
        com: &str,
        rest: &[Cow<'_, str>],
        stdin: impl Into<Stdio>,
        stdout: impl Into<Stdio>,
        stderr: impl Into<Stdio>,
    ) -> Result<Child, Errors> {
        // ugly alloc
        let args: Vec<_> = rest.iter().map(AsRef::as_ref).collect();
        let child = Command::new(com)
            .args(args)
            .stdin(stdin)
            .stdout(stdout)
            .stderr(stderr)
            .spawn()?;

        Ok(child)
    }

    fn run_commands_builtin(
        &mut self,
        completion: &Completion,
        com: Builtins,
        block: args::Command<'_>,
        last_stdout: LastStdout,
    ) -> Result<LastStdout, Errors> {
        if let LastStdout::Child(mut c) = last_stdout {
            // we ignore the output as the builtins doesn't care about it
            let c = c.wait()?;
            let code = c.code().unwrap_or(0);
            if code != 0 {
                return Err(Errors::ExitCode(code));
            }
        }

        let mut redirect = Redirect::new_builtin(block.redirect)?;

        self.run_builtins(
            completion,
            com,
            &block.args,
            &mut redirect.stdout,
            &mut redirect.stderr,
        )?;

        let mut s = redirect.stdout;

        match &mut s {
            RedirectIO::File(file) => {
                file.seek(std::io::SeekFrom::Start(0))?;
                Ok(LastStdout::None)
            }
            RedirectIO::Other(file) => {
                file.seek(std::io::SeekFrom::Start(0))?;
                Ok(LastStdout::Builtin(s))
            }
        }
    }

    fn run_commands_program(
        &mut self,
        block: args::Command<'_>,
        last_stdout: LastStdout,
        is_last: bool,
    ) -> Result<LastStdout, Errors> {
        let redirect = Redirect::new_program(block.redirect, is_last)?;

        let child = self.run_program(
            &block.command,
            &block.args,
            last_stdout,
            redirect.stdout,
            redirect.stderr,
        )?;

        Ok(LastStdout::Child(child))
    }
}

pub fn repl() -> anyhow::Result<Option<ExitCode>> {
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();

    let mut input = String::with_capacity(1024);

    let history = match std::env::var("HISTFILE") {
        Err(_) => history::History::new(),
        Ok(path) => match &path[..] {
            "/home/adrian/.zsh_history" | "/home/adrian/.bash_history" => history::History::new(),
            path => history::History::from_file(path),
        },
    }?;

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

        let completion = Completion::new()?;

        match read_line(&mut input, &mut stdout, &state.history.history, &completion) {
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

        state.history.history.push(input.to_string());

        match state.run_commands(input, &completion) {
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
            Err(e @ Errors::IoError(_)) => {
                writeln!(&stdout, "{}", e)?;
            }
            Err(e @ Errors::ParseError(_)) => {
                writeln!(&stdout, "{}", e)?;
            }
        }

        stdout.flush()?;
        stderr.flush()?;
    }

    Ok(shutdown_code)
}
