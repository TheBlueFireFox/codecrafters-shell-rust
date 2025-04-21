use std::borrow::Cow;

use itertools::Itertools as _;

#[derive(Clone, Copy, Debug)]
enum Character {
    SingleQuote,
    DoubleQuote,
    WhiteSpace,
    Other,
}

impl Character {
    fn map(c: char) -> Self {
        match c {
            '\'' => Self::SingleQuote,
            '"' => Self::DoubleQuote,
            ' ' => Self::WhiteSpace,
            _ => Self::Other,
        }
    }
}

pub fn process_args(args_raw: &str) -> Vec<Cow<'_, str>> {
    let mut v: Vec<Cow<'_, str>> = vec![];

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
                    continue;
                }

                let s = &args_raw[last_idx..=idx];
                v.push(s.into());
                current_block = Character::WhiteSpace;
            }
            (Character::SingleQuote, Character::DoubleQuote) => {
                // case: '" <-
            }
            (Character::SingleQuote, Character::WhiteSpace) => {
                // case: '_ <-
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
                v.push(s.into());
                current_block = Character::WhiteSpace;
            }
            (Character::DoubleQuote, Character::WhiteSpace) => {}
            (Character::DoubleQuote, Character::Other) => {}
            (Character::WhiteSpace, Character::SingleQuote) => {
                // case: _' <-
                current_block = Character::SingleQuote;
                last_idx = idx;
            }
            (Character::WhiteSpace, Character::DoubleQuote) => {
                current_block = Character::DoubleQuote;
                last_idx = idx;
            }
            (Character::WhiteSpace, Character::WhiteSpace) => {}
            (Character::WhiteSpace, Character::Other) => {
                // case: _X <-
                // more special case
                current_block = Character::Other;
                last_idx = idx;
            }
            (Character::Other, Character::SingleQuote) => {
                // case: X' <-
                // b'example'a => bexamplea

                // let s = &args_raw[last_idx..=idx];
                // v.push(s);

                // last_idx = idx;
                current_block = Character::SingleQuote;
            }
            (Character::Other, Character::DoubleQuote) => {
                // case: XX" <-

                // let s = &args_raw[last_idx..=idx];
                // v.push(s.into());

                // last_idx = idx;
                current_block = Character::DoubleQuote;
            }
            (Character::Other, Character::WhiteSpace) => {
                let s = &args_raw[last_idx..idx];
                v.push(s.into());

                current_block = Character::WhiteSpace;
            }
            (Character::Other, Character::Other) => {
                // case: XX <-
            }
        }
    }

    match current_block {
        Character::SingleQuote => {
            unimplemented!("missing end quote")
        }
        Character::DoubleQuote => {
            unimplemented!("missing end double quote")
        }
        Character::WhiteSpace => {}
        Character::Other => {
            let s = &args_raw[last_idx..];
            v.push(s.into());
        }
    }

    for arg in v.iter_mut() {
        // trying to process the argument in a single pass
        let mut it = arg.chars().tuple_windows().enumerate().peekable();

        let mut processing_required = None;

        // check if any of these blocks need processing
        while let Some((idx, (c1, c2))) = it.peek().copied() {
            match (c1, c2) {
                ('\'', _) | ('"', _) | ('\\', _) => {
                    // a ' => needs processing
                    // a " => needs processing
                    // escaped symbol => needs processing
                    processing_required = Some(idx);
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

        let mut last_char = None;
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
                    }
                }
                ('"', _) => match current_context {
                    Character::SingleQuote => {
                        s.push('"');
                    }
                    Character::DoubleQuote => current_context = Character::WhiteSpace,
                    Character::WhiteSpace => current_context = Character::DoubleQuote,
                    Character::Other => {}
                },
                ('\\', '\\') => {
                    s.push('\\');
                    last_char = None;
                    it.next();
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
                            it.next();
                        }
                        _ => {
                            s.push('\\');
                            s.push(x);
                            it.next();
                        }
                    },
                    Character::WhiteSpace => {
                        s.push(x);
                        it.next();
                    }
                    Character::Other => {
                        s.push(x);
                        last_char = None;
                        it.next();
                    }
                },
                v => {
                    // we don't care about this combination
                    s.push(v.0);
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

    v
}

#[cfg(test)]
mod test {
    use super::*;

    use pretty_assertions::assert_str_eq;

    #[test]
    fn simple() {
        let txt = "foo";
        let exp: &[Cow<'_, str>] = &["foo"].map(Into::into);
        let v = process_args(txt);
        for (exp, got) in exp.iter().zip(v) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn simple_multiple() {
        let txt = "foo XX";
        let exp: &[Cow<'_, str>] = &["foo", "XX"].map(Into::into);

        let v = process_args(txt);
        for (exp, got) in exp.iter().zip(v) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn single_single_quote() {
        let txt = "'XX'";
        let exp: &[Cow<'_, str>] = &["XX"].map(Into::into);
        let v = process_args(txt);
        for (exp, got) in exp.iter().zip(v) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn multiple_single_quote() {
        let txt = "'AA    ''BB' 'CC'";
        let exp: &[Cow<'_, str>] = &["AA    BB", "CC"].map(Into::into);
        let v = process_args(txt);
        for (exp, got) in exp.iter().zip(v) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn single_quote_with_double() {
        let txt = "'\"AA\"'";
        let exp: &[Cow<'_, str>] = &["\"AA\""].map(Into::into);
        let v = process_args(txt);
        for (exp, got) in exp.iter().zip(v) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn double_quote() {
        let txt = "\"XX\"";
        let exp: &[Cow<'_, str>] = &["XX"].map(Into::into);
        let v = process_args(txt);
        for (exp, got) in exp.iter().zip(v) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn multiple_double_quote() {
        let txt = "\"AA\"\"BB\" \"CC\"";
        let exp: &[Cow<'_, str>] = &["AABB", "CC"].map(Into::into);
        let v = process_args(txt);
        for (exp, got) in exp.iter().zip(v) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn double_quote_with_single() {
        let txt = "\"'AA'\"";
        let exp: &[Cow<'_, str>] = &["'AA'"].map(Into::into);
        let v = process_args(txt);
        for (exp, got) in exp.iter().zip(v) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn double_backslash() {
        let txt = "/tmp/file\\\\name";
        let exp: &[Cow<'_, str>] = &["/tmp/file\\name"].map(Into::into);
        let v = process_args(txt);
        for (exp, got) in exp.iter().zip(v) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn escaped_string_backslash() {
        let txt = "world\\ \\ \\ \\ \\ \\ script";
        let exp: &[Cow<'_, str>] = &["world      script"].map(Into::into);
        let v = process_args(txt);
        for (exp, got) in exp.iter().zip(v) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn escaped_string_backslash_beginning() {
        let txt = "\\ world";
        let exp: &[Cow<'_, str>] = &[" world"].map(Into::into);
        let v = process_args(txt);
        for (exp, got) in exp.iter().zip(v) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn false_escaped_backslasch() {
        let txt = "\"before\\   after\"";
        let exp: &[Cow<'_, str>] = &["before\\   after"].map(Into::into);
        let v = process_args(txt);
        for (exp, got) in exp.iter().zip(v) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn escaped_before_text() {
        let txt = r#"\'\"world hello\"\'"#;
        let exp: &[Cow<'_, str>] = &[r#"'"world"#, r#"hello"'"#].map(Into::into);
        let v = process_args(txt);
        for (exp, got) in exp.iter().zip(v) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }

    #[test]
    fn backslasch_within_double_quotes() {
        let txt = r#""shell\"insidequotes"example\""#;
        let exp: &[Cow<'_, str>] = &[r#"shell"insidequotesexample""#].map(Into::into);
        let v = process_args(txt);
        for (exp, got) in exp.iter().zip(v) {
            assert_str_eq!(exp.as_ref(), got.as_ref());
        }
    }
}
