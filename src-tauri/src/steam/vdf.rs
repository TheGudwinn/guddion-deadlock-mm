//! Minimal parser for Valve KeyValues (VDF/ACF) text files.

#[derive(Debug, Clone, PartialEq)]
pub enum Vdf {
    Str(String),
    Obj(Vec<(String, Vdf)>),
}

impl Vdf {
    pub fn get(&self, key: &str) -> Option<&Vdf> {
        match self {
            Vdf::Obj(items) => items.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Vdf::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn entries(&self) -> &[(String, Vdf)] {
        match self {
            Vdf::Obj(items) => items,
            _ => &[],
        }
    }
}

#[derive(Debug, PartialEq)]
enum Tok {
    Str(String),
    Open,
    Close,
}

fn tokenize(src: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let mut it = src.chars().peekable();
    while let Some(&c) = it.peek() {
        match c {
            c if c.is_whitespace() => {
                it.next();
            }
            '/' => {
                it.next();
                if it.peek() == Some(&'/') {
                    while let Some(&n) = it.peek() {
                        if n == '\n' {
                            break;
                        }
                        it.next();
                    }
                } else {
                    return Err("unexpected '/'".into());
                }
            }
            '{' => {
                it.next();
                out.push(Tok::Open);
            }
            '}' => {
                it.next();
                out.push(Tok::Close);
            }
            '"' => {
                it.next();
                let mut s = String::new();
                loop {
                    match it.next() {
                        None => return Err("unterminated string".into()),
                        Some('"') => break,
                        Some('\\') => match it.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some('\\') => s.push('\\'),
                            Some('"') => s.push('"'),
                            Some(o) => {
                                s.push('\\');
                                s.push(o);
                            }
                            None => return Err("bad escape".into()),
                        },
                        Some(ch) => s.push(ch),
                    }
                }
                out.push(Tok::Str(s));
            }
            _ => return Err(format!("unexpected character '{c}'")),
        }
    }
    Ok(out)
}

fn parse_obj(toks: &mut std::vec::IntoIter<Tok>, top: bool) -> Result<Vdf, String> {
    let mut items = Vec::new();
    loop {
        match toks.next() {
            None if top => return Ok(Vdf::Obj(items)),
            None => return Err("missing '}'".into()),
            Some(Tok::Close) if !top => return Ok(Vdf::Obj(items)),
            Some(Tok::Close) => return Err("unexpected '}'".into()),
            Some(Tok::Open) => return Err("unexpected '{'".into()),
            Some(Tok::Str(key)) => match toks.next() {
                Some(Tok::Str(v)) => items.push((key, Vdf::Str(v))),
                Some(Tok::Open) => items.push((key, parse_obj(toks, false)?)),
                _ => return Err(format!("missing value for '{key}'")),
            },
        }
    }
}

pub fn parse(src: &str) -> Result<Vdf, String> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let mut toks = tokenize(src)?.into_iter();
    parse_obj(&mut toks, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_with_escapes_and_comments() {
        let v = parse("\"a\"\n{ // c\n \"path\"\t\"C:\\\\Steam\"\n \"apps\" { \"1\" \"2\" } }").unwrap();
        let a = v.get("a").unwrap();
        assert_eq!(a.get("path").unwrap().as_str(), Some("C:\\Steam"));
        assert_eq!(a.get("apps").unwrap().get("1").unwrap().as_str(), Some("2"));
    }

    #[test]
    fn rejects_broken_input() {
        assert!(parse("\"a\" {").is_err());
        assert!(parse("\"a\" \"b").is_err());
    }
}
