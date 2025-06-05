use std::{borrow::Cow, path::PathBuf};

use itertools::Itertools as _;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("No valid Program left of Pipe '|' found")]
    PipeAtBeginning,
    #[error("No valid Program right of Pipe '|' found")]
    PipeAtEnd,
    #[error("A pipe is follwing a pipe?")]
    PipeFollwingPipe,
    #[error("The end double <\"> quote is missing")]
    MissingEndDoubleQuote,
    #[error("The end single quote <'> is missing")]
    MissingEndSingleQuote,
    #[error("No Command found")]
    MissingCommand,
}

pub fn process_args(args: &str) -> Result<Vec<Command<'_>>, Error> {
    let res = process_args_inner(args)?;
    res.into_iter().map(Command::new).collect()
}

fn process_args_inner(args: &str) -> Result<Vec<Vec<Cow<'_, str>>>, Error> {
    // we can assume args_raw has been trimmed of whitespace
    if args.starts_with('|') {
        return Err(Error::PipeAtBeginning);
    }

    let res = split_args(args)?;

    let res = post_process_args(res)?;

    let res = split_pipeline(res)?;

    Ok(res)
}

#[derive(Clone, Copy, Debug)]
enum Character {
    SingleQuote,
    DoubleQuote,
    WhiteSpace,
    Pipe,
    Other,
}

impl Character {
    fn map(c: char) -> Self {
        match c {
            '\'' => Self::SingleQuote,
            '"' => Self::DoubleQuote,
            ' ' => Self::WhiteSpace,
            '|' => Self::Pipe,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Command<'s> {
    pub command: Cow<'s, str>,
    pub args: Vec<Cow<'s, str>>,
    pub redirect: Option<Redirect>,
}

impl<'s> Command<'s> {
    fn new(args: Vec<Cow<'s, str>>) -> Result<Self, Error> {
        let (command, rest) = args.split_first().ok_or(Error::MissingCommand)?;
        let r = Redirect::find_redirect(rest);
        match r {
            Some((i, r)) => Ok(Self {
                command: command.clone(),
                args: rest[..i].to_vec(),
                redirect: Some(r),
            }),
            None => Ok(Self {
                command: command.clone(),
                args: rest.to_vec(),
                redirect: None,
            }),
        }
    }
}

fn split_pipeline(args: Vec<Cow<'_, str>>) -> Result<Vec<Vec<Cow<'_, str>>>, Error> {
    let mut res = Vec::new();

    let mut curr = Vec::new();
    for arg in args {
        if arg == "|" {
            res.push(curr);
            curr = Vec::new();
            continue;
        }
        curr.push(arg);
    }

    res.push(curr);

    Ok(res)
}

fn split_args(args_raw: &str) -> Result<Vec<Cow<'_, str>>, Error> {
    let mut res: Vec<Cow<'_, str>> = vec![];

    let mut current_block = Character::WhiteSpace;
    let mut last_idx = 0;

    let mut it = args_raw.chars().chain([' ']).tuple_windows().enumerate();

    while let Some((idx, (c1, c2))) = it.next() {
        if let ('\\', _) = (c1, c2) {
            // we don't want to skip this one
            if let Character::WhiteSpace = current_block {
                current_block = Character::Other;
                last_idx = idx;
            }
            it.next();
            continue;
        }

        match (current_block, Character::map(c1)) {
            (Character::SingleQuote, Character::SingleQuote) => {
                // case 'XX' <-
                // finished text block
                // + 1 to ignore '
                // ..idx to ignore '
                if !c2.is_whitespace() {
                    current_block = Character::Other;
                    continue;
                }

                let s = &args_raw[last_idx..=idx];
                res.push(s.into());
                current_block = Character::WhiteSpace;
            }
            (Character::SingleQuote, Character::DoubleQuote) => {
                // case: '" <-
            }
            (Character::SingleQuote, Character::WhiteSpace) => {
                // case: '_ <-
            }
            (Character::SingleQuote, Character::Pipe) => {
                // case: 'XX| <-
            }
            (Character::SingleQuote, Character::Other) => {
                // case: 'X <-
            }
            (Character::DoubleQuote, Character::SingleQuote) => {
                // case: "' <-
            }
            (Character::DoubleQuote, Character::DoubleQuote) => {
                // case: "XX" <-
                // finished text block
                // + 1 to ignore '
                // ..idx to ignore '
                if !c2.is_whitespace() {
                    current_block = Character::Other;
                    continue;
                }
                let s = &args_raw[last_idx..=idx];
                res.push(s.into());
                current_block = Character::WhiteSpace;
            }
            (Character::DoubleQuote, Character::WhiteSpace) => {
                // case: "XX _ <-
            }
            (Character::DoubleQuote, Character::Pipe) => {
                // case: "XX| <-
            }
            (Character::DoubleQuote, Character::Other) => {
                // case: "XX <-
            }
            (Character::WhiteSpace, Character::SingleQuote) => {
                // case: _' <-
                current_block = Character::SingleQuote;
                last_idx = idx;
            }
            (Character::WhiteSpace, Character::DoubleQuote) => {
                // case: _"<-
                current_block = Character::DoubleQuote;
                last_idx = idx;
            }
            (Character::WhiteSpace, Character::WhiteSpace) => {
                // case: __<-
            }
            (Character::WhiteSpace, Character::Pipe) => {
                // case: _| <-
                current_block = Character::Pipe;
                last_idx = idx;
            }
            (Character::WhiteSpace, Character::Other) => {
                // case: _X <-
                // more special case
                current_block = Character::Other;
                last_idx = idx;
            }
            (Character::Other, Character::SingleQuote) => {
                // case: X' <-
                current_block = Character::SingleQuote;
            }
            (Character::Other, Character::DoubleQuote) => {
                // case: XX" <-
                current_block = Character::DoubleQuote;
            }
            (Character::Other, Character::WhiteSpace) => {
                let s = &args_raw[last_idx..idx];
                res.push(s.into());

                current_block = Character::WhiteSpace;
            }
            (Character::Other, Character::Other) => {
                // case: XX <-
            }
            (Character::Other, Character::Pipe) => {
                // case: XX| <-
                let s = &args_raw[last_idx..idx];
                res.push(s.into());

                current_block = Character::Pipe;
                last_idx = idx;
            }
            (Character::Pipe, Character::SingleQuote) => {
                let s = &args_raw[last_idx..idx];
                res.push(s.into());

                current_block = Character::SingleQuote;
                last_idx = idx;
            }
            (Character::Pipe, Character::DoubleQuote) => {
                let s = &args_raw[last_idx..idx];
                res.push(s.into());

                current_block = Character::DoubleQuote;
                last_idx = idx;
            }
            (Character::Pipe, Character::WhiteSpace) => {
                let s = &args_raw[last_idx..idx];
                res.push(s.into());

                current_block = Character::WhiteSpace;
                last_idx = idx;
            }
            (Character::Pipe, Character::Pipe) => {
                return Err(Error::PipeFollwingPipe);
            }
            (Character::Pipe, Character::Other) => {
                let s = &args_raw[last_idx..idx];
                res.push(s.into());

                current_block = Character::Other;
                last_idx = idx;
            }
        }
    }

    match current_block {
        Character::SingleQuote => {
            return Err(Error::MissingEndSingleQuote);
        }
        Character::DoubleQuote => {
            return Err(Error::MissingEndDoubleQuote);
        }
        Character::Pipe => {
            return Err(Error::PipeAtEnd);
        }
        Character::Other => {
            let s = &args_raw[last_idx..];
            res.push(s.into());
        }
        Character::WhiteSpace => {}
    }

    Ok(res)
}

fn post_process_args(mut args: Vec<Cow<'_, str>>) -> Result<Vec<Cow<'_, str>>, Error> {
    for arg in args.iter_mut() {
        // trying to process the argument in a single pass
        let mut it = arg.chars().tuple_windows().enumerate().peekable();

        let mut processing_required = None;

        // check if any of these blocks need processing
        let mut last_char = None;
        while let Some((idx, (c1, c2))) = it.peek() {
            last_char = Some(*c2);
            match (c1, c2) {
                ('\'', _) | ('"', _) | ('\\', _) => {
                    // a ' => needs processing
                    // a " => needs processing
                    // escaped symbol => needs processing
                    processing_required = Some(*idx);
                    break;
                }
                _ => {
                    // we don't care about this combination
                }
            }
            // consume the token
            it.next();
        }

        let mut s = match processing_required {
            None => continue,
            Some(c) => {
                let mut s = String::with_capacity(arg.len());
                // add the clean blocks
                s.push_str(&arg[..c]);
                s
            }
        };

        let mut current_context = Character::WhiteSpace;

        while let Some((_, (c1, c2))) = it.next() {
            last_char = Some(c2);
            match (c1, c2) {
                ('\'', _) => {
                    // a ' => needs processing
                    // a " => needs processing
                    // escaped symbol => needs processing
                    match current_context {
                        Character::SingleQuote => {
                            current_context = Character::WhiteSpace;
                        }
                        Character::DoubleQuote => {
                            s.push('\'');
                        }
                        Character::WhiteSpace => {
                            current_context = Character::SingleQuote;
                        }
                        Character::Other => {
                            unimplemented!("If I get this one I messed up");
                        }
                        Character::Pipe => {
                            unreachable!("We should have errored out earlier for this case")
                        }
                    }
                }
                ('"', _) => match current_context {
                    Character::SingleQuote => {
                        s.push('"');
                    }
                    Character::DoubleQuote => current_context = Character::WhiteSpace,
                    Character::WhiteSpace => current_context = Character::DoubleQuote,
                    Character::Other => {}
                    Character::Pipe => {
                        unreachable!("We should have errored out earlier for this case")
                    }
                },
                ('\\', '\\') => {
                    s.push('\\');
                    last_char = it.next().map(|(_, (_, x))| x);
                }
                ('\\', x) => match current_context {
                    Character::SingleQuote => {
                        s.push('\\');
                        s.push(x);
                        it.next();
                    }
                    Character::DoubleQuote => match x {
                        '"' => {
                            s.push('"');
                            last_char = it.next().map(|(_, (_, x))| x);
                        }
                        _ => {
                            s.push('\\');
                            s.push(x);
                            last_char = it.next().map(|(_, (_, x))| x);
                        }
                    },
                    Character::WhiteSpace => {
                        s.push(x);
                        last_char = it.next().map(|(_, (_, x))| x);
                    }
                    Character::Other => {
                        s.push(x);
                        last_char = it.next().map(|(_, (_, x))| x);
                    }
                    Character::Pipe => {
                        unreachable!("We should have errored out earlier for this case")
                    }
                },
                (v, _) => {
                    // we don't care about this combination
                    s.push(v);
                }
            }
        }

        match last_char {
            Some('\'' | '"' | '\\') => {}
            Some(x) => s.push(x),
            None => {}
        }

        *arg = Cow::Owned(s);
    }

    Ok(args)
}

#[derive(Clone, Debug)]
pub enum RedirectIO {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug)]
pub struct Redirect {
    pub to: RedirectIO,
    pub append: bool,
    pub file_path: PathBuf,
}

impl Redirect {
    fn find_redirect(args: &[Cow<'_, str>]) -> Option<(usize, Self)> {
        for (i, (operator, file_path)) in args.iter().tuple_windows().enumerate() {
            let s = match operator.as_ref() {
                "1>" | ">" => Self {
                    to: RedirectIO::Stdout,
                    append: false,
                    file_path: file_path.as_ref().into(),
                },
                "2>" => Self {
                    to: RedirectIO::Stderr,
                    append: false,
                    file_path: file_path.as_ref().into(),
                },
                "1>>" | ">>" => Self {
                    to: RedirectIO::Stdout,
                    append: true,
                    file_path: file_path.as_ref().into(),
                },
                "2>>" => Self {
                    to: RedirectIO::Stderr,
                    append: true,
                    file_path: file_path.as_ref().into(),
                },
                "|" => {
                    // redirect
                    unreachable!("There should never be a random pipe here")
                }
                _ => {
                    continue;
                }
            };

            return Some((i, s));
        }
        None
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use pretty_assertions::assert_str_eq;

    #[test]
    fn simple() {
        let txt = "foo";
        let exp: &[Cow<'_, str>] = &["foo"].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn simple_multiple() {
        let txt = "foo XX";
        let exp: &[Cow<'_, str>] = &["foo", "XX"].map(Into::into);

        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn single_single_quote() {
        let txt = "'XX'";
        let exp: &[Cow<'_, str>] = &["XX"].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn multiple_single_quote() {
        let txt = "'AA    ''BB' 'CC'";
        let exp: &[Cow<'_, str>] = &["AA    BB", "CC"].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn multiple_single_quote_mixed() {
        let txt = r#"echo 'example     script' 'shell''test' hello''world"#;
        let exp: &[Cow<'_, str>] = &["echo","example     script", "shelltest", "helloworld"].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn single_quote_with_double() {
        let txt = "'\"AA\"'";
        let exp: &[Cow<'_, str>] = &["\"AA\""].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn double_quote() {
        let txt = "\"XX\"";
        let exp: &[Cow<'_, str>] = &["XX"].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn multiple_double_quote() {
        let txt = "\"AA\"\"BB\" \"CC\"";
        let exp: &[Cow<'_, str>] = &["AABB", "CC"].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn double_quote_with_single() {
        let txt = "\"'AA'\"";
        let exp: &[Cow<'_, str>] = &["'AA'"].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn double_backslash() {
        let txt = "/tmp/file\\\\name";
        let exp: &[Cow<'_, str>] = &["/tmp/file\\name"].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn escaped_string_backslash() {
        let txt = "world\\ \\ \\ \\ \\ \\ script";
        let exp: &[Cow<'_, str>] = &["world      script"].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn escaped_string_backslash_beginning() {
        let txt = "\\ world";
        let exp: &[Cow<'_, str>] = &[" world"].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn false_escaped_backslasch() {
        let txt = "\"before\\   after\"";
        let exp: &[Cow<'_, str>] = &["before\\   after"].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn escaped_before_text() {
        let txt = r#"\'\"world hello\"\'"#;
        let exp: &[Cow<'_, str>] = &[r#"'"world"#, r#"hello"'"#].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn backslasch_within_double_quotes() {
        let txt = r#""shell\"insidequotes"example\""#;
        let exp: &[Cow<'_, str>] = &[r#"shell"insidequotesexample""#].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn simple_constalation() {
        let txt = r#"'exe  with  space'"#;

        let exp: &[Cow<'_, str>] = &[r#"exe  with  space"#].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn simple_escape() {
        let txt = r#""/tmp/baz/f\1" "#;
        let exp: &[Cow<'_, str>] = &["/tmp/baz/f\\1"].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn multiple_simple_escape() {
        let txt = r#"cat "/tmp/baz/f\n37" "/tmp/baz/f\1" "/tmp/baz/f'\'31""#;
        let exp: &[Cow<'_, str>] = &[
            "cat",
            "/tmp/baz/f\\n37",
            "/tmp/baz/f\\1",
            "/tmp/baz/f'\\'31",
        ]
        .map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn simple_pipe_single_quote() {
        let txt = r#"'exe with |'"#;

        let exp: &[Cow<'_, str>] = &[r#"exe with |"#].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn simple_pipe_double_quote() {
        let txt = r#""exe with |""#;

        let exp: &[Cow<'_, str>] = &[r#"exe with |"#].map(Into::into);
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(1, v.len());
        for (exp, got) in exp.iter().zip(&v[0]) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn pipe_with_space() {
        let txt = r#"exe | exe2"#;

        let exp: &[&[&str]] = &[&["exe"], &["exe2"]];
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(2, v.len());
        for (block, v) in exp.iter().zip(v) {
            for (&exp, got) in block.iter().zip(v) {
                assert_str_eq!(exp, got.as_ref());
            }
        }
    }

    #[test]
    fn pipe_with_space_left() {
        let txt = r#"exe| exe2"#;

        let exp: &[&[&str]] = &[&["exe"], &["exe2"]];
        let v = process_args_inner(txt).expect("able to parse");

        assert_eq!(2, v.len());
        for (block, v) in exp.iter().zip(v) {
            for (&exp, got) in block.iter().zip(v) {
                assert_str_eq!(exp, got.as_ref());
            }
        }
    }

    #[test]
    fn pipe_beween_commands() {
        let txt = r#"exe|exe2"#;

        let exp: &[&[&str]] = &[&["exe"], &["exe2"]];
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(2, v.len());
        for (block, v) in exp.iter().zip(v) {
            for (&exp, got) in block.iter().zip(v) {
                assert_str_eq!(exp, got.as_ref());
            }
        }
    }

    #[test]
    fn pipe_beween_commands_with_args() {
        let txt = r#"exe foo |exe2"#;

        let exp: &[&[&str]] = &[&["exe", "foo"], &["exe2"]];
        let v = process_args_inner(txt).expect("able to parse");
        assert_eq!(2, v.len());
        for (block, v) in exp.iter().zip(v) {
            for (&exp, got) in block.iter().zip(v) {
                assert_str_eq!(exp, got.as_ref());
            }
        }
    }

    #[test]
    fn pipe_at_beginning() {
        let txt = r#"|exe"#;

        let v = process_args_inner(txt);
        assert_eq!(Err(Error::PipeAtBeginning), v);
    }

    #[test]
    fn pipe_at_end() {
        let txt = r#"exe|"#;

        let v = process_args_inner(txt);
        assert_eq!(Err(Error::PipeAtEnd), v);
    }
}
