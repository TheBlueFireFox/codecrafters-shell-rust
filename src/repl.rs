use std::{
    borrow::Cow,
    io::{Read, Seek, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    str::FromStr,
};

use anyhow::Context;

use memfile::MemFile;

use crate::{
    args,
    builtin::{Builtins, Errors, ExitCode},
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

    fn run_pwd<'name>(&self, stdout: &mut dyn std::io::Write) -> Result<(), Errors<'name>> {
        let p = format!("{:?}", self.path);
        writeln!(stdout, "{}", p.trim_matches('"'))?;
        Ok(())
    }

    fn run_history<'name>(
        &self,
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
        &self,
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
        &self,
        rest: &[Cow<'name, str>],
        stdout: &mut dyn std::io::Write,
    ) -> Result<(), Errors<'name>> {
        writeln!(stdout, "{}", rest.join(" "))?;
        Ok(())
    }

    fn run_exit<'name>(&self, rest: &[Cow<'name, str>]) -> Result<(), Errors<'name>> {
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
        stdin: impl Into<Stdio>,
        stdout: impl Into<Stdio>,
        stderr: impl Into<Stdio>,
    ) -> Result<Child, Errors<'com>> {
        let _path = Self::is_program(com)?;

        // ugly alloc
        let args: Vec<_> = rest.iter().map(AsRef::as_ref).collect();
        let child = Command::new(com.as_ref())
            .args(args)
            .stdin(stdin)
            .stdout(stdout)
            .stderr(stderr)
            .spawn()?;

        Ok(child)
    }

    fn run_commands<'com>(&mut self, command: &'com str) -> Result<(), Errors<'com>> {
        let args = args::process_args(command)?;

        let mut last_stdout = LastStdout::None;
        let blocks = args.len();

        for (i, block) in args.into_iter().enumerate() {
            let is_last = i + 1 == blocks;

            last_stdout = match block.command.as_ref().try_into() {
                Ok(com) => self.run_commands_builtin(com, block, last_stdout),
                Err(_) => self.run_commands_program(block, last_stdout, is_last),
            }?;
        }

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
            LastStdout::Builtin(redirect_io) => {
                match redirect_io {
                    RedirectIO::Other(mut m) => {
                        let mut buf = String::new();
                        m.read_to_string(&mut buf)?;
                        print!("{}", buf);
                    }
                    RedirectIO::File(mut f) => {
                        let mut buf = String::new();
                        f.read_to_string(&mut buf)?;
                        print!("{}", buf);
                    }
                }
                Ok(())
            }
        }
    }

    fn run_commands_builtin<'com>(
        &mut self,
        com: Builtins,
        block: args::Command<'com>,
        last_stdout: LastStdout,
    ) -> Result<LastStdout, Errors<'com>> {
        if let LastStdout::Child(mut c) = last_stdout {
            // we ignore the output as the builtins doesn't care about it
            let c = c.wait()?;
            let code = c.code().unwrap_or(0);
            if code != 0 {
                return Err(Errors::ExitCode(code));
            }
        }

        let mut redirect = Redirect::new_builtin(block.redirect)?;

        self.run_builtins(com, &block.args, &mut redirect.stdout, &mut redirect.stderr)?;

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

    fn run_commands_program<'com>(
        &mut self,
        block: args::Command<'com>,
        last_stdout: LastStdout,
        is_last: bool,
    ) -> Result<LastStdout, Errors<'com>> {
        let redirect = Redirect::new_program(block.redirect, is_last)?;

        match self.run_program(
            &block.command,
            &block.args,
            last_stdout,
            redirect.stdout,
            redirect.stderr,
        ) {
            Ok(child) => Ok(LastStdout::Child(child)),
            Err(Errors::CommandNotFound(_)) => Err(Errors::CommandNotFound(block.command.clone())),
            Err(err) => Err(err),
        }
    }
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
            Err(e @ Errors::ParseError(_)) => {
                writeln!(&stdout, "{}", e)?;
            }
        }
    }

    Ok(shutdown_code)
}
