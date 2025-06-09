use std::{ffi::OsStr, path::PathBuf};

use trie_rs::map::TrieBuilder;

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
pub enum Type {
    Builtin(Builtins),
    Program(PathBuf),
}

#[derive(Debug, Clone)]
pub struct Completion {
    map: trie_rs::map::Trie<u8, Type>,
}

impl Completion {
    pub fn new() -> Result<Self, CompletionError> {
        let mut builder = trie_rs::map::TrieBuilder::new();

        Self::generate_builtins(&mut builder)?;

        // program names
        Self::generate_program_names(&mut builder)?;
        let s = Self {
            map: builder.build(),
        };

        Ok(s)
    }

    pub fn matches_exact(&self, query: impl AsRef<str>) -> Option<&Type> {
        self.map.exact_match(query.as_ref().as_bytes())
    }

    pub fn matches(&self, query: impl AsRef<str>) -> impl Iterator<Item = (String, &Type)> {
        self.map.predictive_search(query.as_ref().as_bytes())
    }

    pub fn longest_prefix(&self, query: impl AsRef<str>) -> Option<String> {
        self.map.longest_prefix(query.as_ref())
    }

    fn generate_builtins(builder: &mut TrieBuilder<u8, Type>) -> Result<(), CompletionError> {
        for (b, n) in Builtins::supported() {
            builder.push(n, Type::Builtin(b));
        }
        Ok(())
    }

    fn generate_program_names(builder: &mut TrieBuilder<u8, Type>) -> Result<(), CompletionError> {
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
                builder.push(name, Type::Program(file));
            }
        }

        Ok(())
    }
}
