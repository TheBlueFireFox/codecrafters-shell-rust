use std::{ffi::OsStr, path::PathBuf};

use crate::builtin::Builtins;

pub type Completion = trie_rs::Trie<u8>;

#[derive(Debug, thiserror::Error)]
pub enum TabHandlingError {
    #[error("PATH env is not set")]
    MissingPathEnv,
    #[error("File in PATH has not existing path name <{0}>")]
    FileNameMissing(PathBuf),
    #[error("Io Error <{0}>")]
    Io(#[from] std::io::Error),
}

fn generate_program_names() -> Result<Vec<String>, TabHandlingError> {
    let mut v = vec![];
    let paths = std::env::var("PATH").map_err(|_| TabHandlingError::MissingPathEnv)?;

    let process_file = |file: PathBuf| {
        file.file_name()
            .and_then(OsStr::to_str)
            .map(str::to_owned)
            .ok_or(TabHandlingError::FileNameMissing(file))
    };

    for path in std::env::split_paths(&paths).filter(|e| e.exists()) {
        for file in std::fs::read_dir(path)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|e| e.is_file())
        {
            let name = process_file(file)?;
            v.push(name);
        }
    }

    Ok(v)
}

pub fn generate_completion() -> Result<Completion, TabHandlingError> {
    let mut builder = trie_rs::TrieBuilder::new();

    // add builtins
    for s in Builtins::supported() {
        builder.push(s);
    }

    // program names
    let exes = generate_program_names()?;
    for s in exes {
        builder.push(s);
    }

    Ok(builder.build())
}
