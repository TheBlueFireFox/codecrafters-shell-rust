use std::{fs::File, process::Stdio};

use memfile::MemFile;

use crate::args;

pub enum RedirectIO<T> {
    File(File),
    Other(T),
}

impl<T> From<RedirectIO<T>> for Stdio
where
    T: Into<Stdio>,
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
    T: std::io::Write,
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

pub struct Redirect<T> {
    pub stdout: RedirectIO<T>,
    pub stderr: RedirectIO<T>,
}

impl Redirect<MemFile> {
    pub fn new_builtin(redirect: Option<args::Redirect>) -> std::io::Result<Self> {
        // the last child may directly write to stdout
        match redirect {
            Some(redirect) => Self::new_program_with_redirect(redirect),
            None => Self::new_program_no_redirect(),
        }
    }

    fn new_program_no_redirect() -> std::io::Result<Self> {
        let stderr = MemFile::create_default("RedirectFileStdErr")?;
        let stdout = MemFile::create_default("RedirectFileStdOut")?;

        Ok(Self {
            stdout: RedirectIO::Other(stdout),
            stderr: RedirectIO::Other(stderr),
        })
    }

    fn new_program_with_redirect(redirect: args::Redirect) -> std::io::Result<Self> {
        let mut opts = File::options();
        opts.create(true).read(true);

        if redirect.append {
            opts.append(true);
        } else {
            opts.truncate(true).write(true);
        }

        let file = opts.open(redirect.file_path)?;

        let s = match redirect.to {
            args::RedirectIO::Stdout => {
                let stderr = MemFile::create_default("RedirectFileStdErr")?;
                Self {
                    stdout: RedirectIO::File(file),
                    stderr: RedirectIO::Other(stderr),
                }
            }
            args::RedirectIO::Stderr => {
                let stdout = MemFile::create_default("RedirectFileStdOut")?;
                Self {
                    stdout: RedirectIO::Other(stdout),
                    stderr: RedirectIO::File(file),
                }
            }
        };
        Ok(s)
    }
}

impl Redirect<Stdio> {
    pub fn new_program(redirect: Option<args::Redirect>, is_last: bool) -> std::io::Result<Self> {
        // the last child may directly write to stdout
        match redirect {
            Some(redirect) => Self::new_program_with_redirect(redirect, is_last),
            None => Self::new_program_no_redirect(is_last),
        }
    }

    fn is_last<S: Into<Stdio>>(is_last: bool, s: impl Fn() -> S) -> Stdio {
        if is_last {
            s().into()
        } else {
            Stdio::piped()
        }
    }

    fn new_program_with_redirect(redirect: args::Redirect, is_last: bool) -> std::io::Result<Self> {
        let mut opts = File::options();
        opts.create(true).write(true);

        if redirect.append {
            opts.truncate(false).append(true);
        } else {
            opts.truncate(true);
        }

        let file = opts.open(redirect.file_path)?;

        let s = match redirect.to {
            args::RedirectIO::Stdout => {
                let stderr = Self::is_last(is_last, std::io::stderr);
                Self {
                    stdout: RedirectIO::Other(Stdio::from(file)),
                    stderr: RedirectIO::Other(stderr),
                }
            }
            args::RedirectIO::Stderr => {
                let stdout = Self::is_last(is_last, std::io::stdout);
                Self {
                    stdout: RedirectIO::Other(stdout),
                    stderr: RedirectIO::Other(Stdio::from(file)),
                }
            }
        };
        Ok(s)
    }

    fn new_program_no_redirect(is_last: bool) -> std::io::Result<Self> {
        let stdout = Self::is_last(is_last, std::io::stdout);
        let stderr = Self::is_last(is_last, std::io::stderr);

        Ok(Self {
            stdout: RedirectIO::Other(stdout),
            stderr: RedirectIO::Other(stderr),
        })
    }
}
