use std::fmt;

use crate::{Error, parse::consts::MAX_TYPE_DEPTH};

macro_rules! keywords {
    ($($variant:ident = $text:literal,)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Kw {
            $($variant,)*
        }

        impl Kw {
            #[cfg(test)]
            pub const ALL: &[(Self, &str)] = &[$((Self::$variant, $text),)*];

            fn classify(word: &str) -> Option<Self> {
                hashify::map! {
                    word.as_bytes(),
                    Kw,
                    $($text => Kw::$variant,)*
                }
                .copied()
            }
        }
    };
}

keywords! {
    String = "String",
    Bool = "Bool",
    UInt8 = "UInt8",
    UInt16 = "UInt16",
    UInt32 = "UInt32",
    UInt64 = "UInt64",
    UInt128 = "UInt128",
    UInt256 = "UInt256",
    Int8 = "Int8",
    Int16 = "Int16",
    Int32 = "Int32",
    Int64 = "Int64",
    Int128 = "Int128",
    Int256 = "Int256",
    Float32 = "Float32",
    Float64 = "Float64",
    BFloat16 = "BFloat16",
    Date = "Date",
    Date32 = "Date32",
    Time = "Time",
    LineString = "LineString",
    MultiLineString = "MultiLineString",
    MultiPolygon = "MultiPolygon",
    MultiPoint = "MultiPoint",
    Polygon = "Polygon",
    Ring = "Ring",
    Point = "Point",
    Geometry = "Geometry",
    SharedVariant = "SharedVariant",
    Nothing = "Nothing",
    Uuid = "UUID",
    Ipv4 = "IPv4",
    Ipv6 = "IPv6",
    IntervalNanosecond = "IntervalNanosecond",
    IntervalMicrosecond = "IntervalMicrosecond",
    IntervalMillisecond = "IntervalMillisecond",
    IntervalSecond = "IntervalSecond",
    IntervalMinute = "IntervalMinute",
    IntervalHour = "IntervalHour",
    IntervalDay = "IntervalDay",
    IntervalWeek = "IntervalWeek",
    IntervalMonth = "IntervalMonth",
    IntervalQuarter = "IntervalQuarter",
    IntervalYear = "IntervalYear",
    FixedString = "FixedString",
    Decimal = "Decimal",
    Decimal32 = "Decimal32",
    Decimal64 = "Decimal64",
    Decimal128 = "Decimal128",
    Decimal256 = "Decimal256",
    DateTime = "DateTime",
    DateTime64 = "DateTime64",
    Time64 = "Time64",
    Nullable = "Nullable",
    LowCardinality = "LowCardinality",
    Array = "Array",
    Map = "Map",
    Tuple = "Tuple",
    Nested = "Nested",
    Variant = "Variant",
    Enum8 = "Enum8",
    Enum16 = "Enum16",
    Json = "JSON",
    Dynamic = "Dynamic",
    SimpleAggregateFunction = "SimpleAggregateFunction",
    AggregateFunction = "AggregateFunction",
    QBit = "QBit",
    MaxTypes = "max_types",
    MaxDynamicPaths = "max_dynamic_paths",
    MaxDynamicTypes = "max_dynamic_types",
    Skip = "SKIP",
    Regexp = "REGEXP",
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok<'a> {
    LParen,
    RParen,
    Comma,
    Eq,
    Space,
    Number(&'a str),
    Negative(&'a str),
    Quoted(&'a str),
    Backquoted(&'a str),
    Word(&'a str),
    Path(&'a str),
    Kw(Kw, &'a str),
}

impl fmt::Display for Tok<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match *self {
            Self::LParen => "(",
            Self::RParen => ")",
            Self::Comma => ", ",
            Self::Eq => "=",
            Self::Space => " ",
            Self::Number(s)
            | Self::Negative(s)
            | Self::Quoted(s)
            | Self::Backquoted(s)
            | Self::Word(s)
            | Self::Path(s)
            | Self::Kw(_, s) => s,
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Lexeme<'a> {
    start_byte: usize,
    token: Tok<'a>,
    end_byte: usize,
}

impl<'a> From<Lexeme<'a>> for (usize, Tok<'a>, usize) {
    fn from(lexeme: Lexeme<'a>) -> Self {
        (lexeme.start_byte, lexeme.token, lexeme.end_byte)
    }
}

pub struct Lexer<'a> {
    text: &'a str,
    pos: usize,
    depth: usize,
}

impl<'a> Lexer<'a> {
    pub const fn new(text: &'a str) -> Self {
        Self {
            text,
            pos: 0,
            depth: 0,
        }
    }
}

impl<'a> Iterator for Lexer<'a> {
    type Item = Result<Lexeme<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let bytes = self.text.as_bytes();
        let start = self.pos;
        let &first = bytes.get(start)?;
        let (tok, end) = match first {
            b'(' => {
                self.depth += 1;
                if self.depth > MAX_TYPE_DEPTH {
                    self.pos = bytes.len();
                    return Some(Err(Error::Parse(format!(
                        "type nesting exceeds {MAX_TYPE_DEPTH} levels"
                    ))));
                }
                (Tok::LParen, start + 1)
            }
            b')' => {
                // Let the grammar reject unmatched closing parentheses without underflowing here.
                self.depth = self.depth.saturating_sub(1);
                (Tok::RParen, start + 1)
            }
            b',' => {
                if bytes.get(start + 1) != Some(&b' ') {
                    self.pos = bytes.len();
                    return Some(Err(Error::Parse(format!("expected `, ` at byte {start}"))));
                }
                (Tok::Comma, start + 2)
            }
            b'=' => (Tok::Eq, start + 1),
            b' ' => (Tok::Space, start + 1),
            b'\'' | b'`' => {
                let Some(end) = quoted_end(bytes, start) else {
                    self.pos = bytes.len();
                    return Some(Err(Error::Parse(format!(
                        "unterminated quoted name at byte {start}"
                    ))));
                };
                let text = &self.text[start..end];
                let tok = if first == b'\'' {
                    Tok::Quoted(text)
                } else {
                    Tok::Backquoted(text)
                };
                (tok, end)
            }
            _ => {
                let mut end = start + 1;
                while bytes.get(end).is_some_and(|&byte| !is_delimiter(byte)) {
                    end += 1;
                }
                (classify(&self.text[start..end]), end)
            }
        };
        self.pos = end;
        Some(Ok(Lexeme {
            start_byte: start,
            token: tok,
            end_byte: end,
        }))
    }
}

const fn is_delimiter(byte: u8) -> bool {
    matches!(byte, b' ' | b'(' | b')' | b',' | b'=' | b'\'' | b'`')
}

// A doubled quote can continue a literal only if a later quote closes it.
// Remember the last valid close so an unmatched continuation remains input for
// the lexer to reject rather than being accepted as part of the name.
fn quoted_end(bytes: &[u8], open: usize) -> Option<usize> {
    let quote = bytes[open];
    let mut end = None;
    let mut index = open + 1;
    while let Some(&byte) = bytes.get(index) {
        if byte == b'\\' {
            // The escaped char may be multi-byte; its continuation bytes are
            // never a quote or backslash, so skipping the lead byte suffices.
            index += 2;
        } else if byte == quote {
            end = Some(index + 1);
            if bytes.get(index + 1) != Some(&quote) {
                break;
            }
            index += 2;
        } else {
            index += 1;
        }
    }
    end
}

fn classify(run: &str) -> Tok<'_> {
    let bytes = run.as_bytes();
    if bytes.iter().all(u8::is_ascii_digit) {
        Tok::Number(run)
    } else if bytes[0] == b'-' && bytes.len() > 1 && bytes[1..].iter().all(u8::is_ascii_digit) {
        Tok::Negative(run)
    } else if bytes
        .iter()
        .all(|&byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        Kw::classify(run).map_or(Tok::Word(run), |kw| Tok::Kw(kw, run))
    } else {
        Tok::Path(run)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(text: &str) -> Vec<Tok<'_>> {
        Lexer::new(text)
            .map(|item| item.map(|lexeme| lexeme.token))
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn classifies_runs_by_longest_match() {
        assert_eq!(
            toks("Tuple(a.b UInt8, -5 1x -)"),
            vec![
                Tok::Kw(Kw::Tuple, "Tuple"),
                Tok::LParen,
                Tok::Path("a.b"),
                Tok::Space,
                Tok::Kw(Kw::UInt8, "UInt8"),
                Tok::Comma,
                Tok::Negative("-5"),
                Tok::Space,
                Tok::Word("1x"),
                Tok::Space,
                Tok::Path("-"),
                Tok::RParen,
            ]
        );
    }

    #[test]
    fn keyword_near_misses_remain_identifiers() {
        for word in [
            "uint8",
            "UInt8x",
            "UInt",
            "_String",
            "StrinG",
            "IntervalNanoseconD",
            "IntervalNanosecond_",
            "max_types_",
            "MAX_TYPES",
        ] {
            assert_eq!(toks(word), vec![Tok::Word(word)], "{word}");
        }
    }

    #[test]
    fn whitespace_is_positional() {
        assert_eq!(
            toks("a  b"),
            vec![Tok::Word("a"), Tok::Space, Tok::Space, Tok::Word("b")]
        );
        assert_eq!(toks("a\tb"), vec![Tok::Path("a\tb")]);
        assert!(Lexer::new("a,b").any(|item| item.is_err()));
        assert!(Lexer::new("a,").any(|item| item.is_err()));
    }

    #[test]
    fn doubled_quote_continues_when_it_can_still_close() {
        assert_eq!(toks(r"'a''b'"), vec![Tok::Quoted(r"'a''b'")]);
        assert_eq!(
            toks(r"'a' 'b'"),
            vec![Tok::Quoted("'a'"), Tok::Space, Tok::Quoted("'b'")]
        );
        assert_eq!(
            toks(r"'a\'' x"),
            vec![Tok::Quoted(r"'a\''"), Tok::Space, Tok::Word("x")]
        );
        let mut lexer = Lexer::new("`a`` UInt8");
        assert_eq!(
            lexer.next().unwrap().unwrap(),
            Lexeme {
                start_byte: 0,
                token: Tok::Backquoted("`a`"),
                end_byte: 3,
            }
        );
        assert!(lexer.next().unwrap().is_err());
    }

    #[test]
    fn unterminated_quote_is_an_error() {
        assert!(Lexer::new("`abc").any(|item| item.is_err()));
        assert!(Lexer::new(r"'abc\'").any(|item| item.is_err()));
    }
}
