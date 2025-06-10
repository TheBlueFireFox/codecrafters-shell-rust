use std::{ffi::OsStr, path::PathBuf};

use ptrie::Trie;

use crate::builtin::Builtins;

#[derive(Debug, thiserror::Error)]
pub enum CompletionError {
    #[error("PATH env is not set")]
    MissingPathEnv,
    #[error("File in PATH has not existing path name <{0}>")]
    FileNameMissing(PathBuf),
    #[error("Io Error <{0}>")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub key: String,
    pub value: Type,
}

#[derive(Debug, Clone)]
pub enum Type {
    Builtin(Builtins),
    Program(PathBuf),
}

#[derive(Debug, Clone)]
pub struct Completion {
    map: Trie<u8, Entry>,
}

impl Completion {
    pub fn new() -> Result<Self, CompletionError> {
        let mut trie = Trie::new();

        Self::generate_builtins(&mut trie)?;

        // program names
        Self::generate_program_names(&mut trie)?;
        let s = Self { map: trie };

        Ok(s)
    }

    pub fn matches_exact(&self, query: impl AsRef<str>) -> Option<&Entry> {
        self.map.get(query.as_ref().bytes())
    }

    pub fn matches(&self, query: impl AsRef<str>) -> Vec<&Entry> {
        self.map.find_postfixes(query.as_ref().bytes())
    }

    pub fn longest_prefix(&self, query: impl AsRef<str>) -> Option<&Entry> {
        self.map.find_longest_prefix(query.as_ref().bytes())
    }

    fn generate_builtins(builder: &mut Trie<u8, Entry>) -> Result<(), CompletionError> {
        for (b, n) in Builtins::supported() {
            builder.insert(
                n.bytes(),
                Entry {
                    key: n.into(),
                    value: Type::Builtin(b),
                },
            );
        }
        Ok(())
    }

    fn generate_program_names(builder: &mut Trie<u8, Entry>) -> Result<(), CompletionError> {
        let paths = std::env::var("PATH").map_err(|_| CompletionError::MissingPathEnv)?;

        let process_file = |file: &PathBuf| {
            file.file_name()
                .and_then(OsStr::to_str)
                .map(str::to_owned)
                .ok_or_else(|| CompletionError::FileNameMissing(file.clone()))
        };

        for path in std::env::split_paths(&paths).filter(|e| e.exists()) {
            for file in std::fs::read_dir(path)?
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|e| e.is_file())
            {
                let name = process_file(&file)?;
                builder.insert(
                    name.bytes(),
                    Entry {
                        key: name.clone(),
                        value: Type::Program(file),
                    },
                );
            }
        }

        Ok(())
    }
}
