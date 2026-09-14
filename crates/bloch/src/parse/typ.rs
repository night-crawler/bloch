use std::{borrow::Cow, collections::HashSet};

use bstr::ByteSlice as _;

use crate::{Error, types::Type};

mod lexer;

lalrpop_util::lalrpop_mod!(
    #[allow(
        unused_imports,
        clippy::all,
        clippy::pedantic,
        clippy::nursery,
        clippy::restriction
    )]
    type_header,
    "/parse/type_header.rs"
);

impl Type<'_> {
    pub(crate) fn from_bytes(input: &[u8]) -> crate::Result<Type<'_>> {
        let input = crate::error::decode_utf8(input)?;
        type_header::TypeParser::new()
            .parse(lexer::Lexer::new(input))
            .map_err(|error| match error {
                lalrpop_util::ParseError::User { error } => error,
                error => Error::Parse(error.to_string()),
            })
    }

    /// ClickHouse code: `DataTypeDecimal`. Buckets `Decimal(P, S)` by precision into the
    /// four storage widths; `Decimal32(S)` and friends are the same buckets with a fixed `P`.
    fn decimal(precision: u8, scale: u8) -> crate::Result<Type<'static>> {
        if scale > precision {
            return Err(Error::Parse("decimal scale exceeds precision".into()));
        }
        match precision {
            0..10 => Ok(Type::Decimal32(scale)),
            10..19 => Ok(Type::Decimal64(scale)),
            19..39 => Ok(Type::Decimal128(scale)),
            39..77 => Ok(Type::Decimal256(scale)),
            _ => Err(Error::Parse("decimal precision exceeds 76".into())),
        }
    }
}

/// ClickHouse code: `EnumValues::EnumValues` sorts values by numeric id and rejects
/// duplicate ids and names, and `DataTypeEnum::generateName` writes them in that order.
/// `mark::Enum8::name`/`mark::Enum16::name` binary-search `variants` by id, so the header
/// must already satisfy the ordering.
fn enum_variants<T: PartialOrd>(
    pairs: Vec<(Cow<'_, str>, T)>,
) -> crate::Result<Vec<(Cow<'_, str>, T)>> {
    if !pairs.windows(2).all(|pair| pair[0].1 < pair[1].1) {
        return Err(Error::Parse(
            "enum values must be strictly increasing".into(),
        ));
    }
    if pairs.len() > 1 {
        let mut names = HashSet::with_capacity(pairs.len());
        if pairs.iter().any(|(name, _)| !names.insert(name.as_ref())) {
            return Err(Error::Parse("duplicate enum label".into()));
        }
    }
    Ok(pairs)
}

/// Decodes a `'…'` or `` `…` `` literal, `token` including both delimiting quotes.
///
/// ClickHouse code: `readAnyQuotedStringInto` (doubled quote → one quote) and
/// `parseComplexEscapeSequence` in `src/IO/ReadHelpers.cpp`. Invalid hex digits after
/// `\x` are rejected where ClickHouse's `unhex2` would produce garbage.
fn quoted(token: &str) -> crate::Result<Cow<'_, str>> {
    let quote = token.as_bytes()[0];
    let text = &token[1..token.len() - 1];
    let escapes = [b'\\', quote];
    let mut bytes = text.as_bytes();
    let Some(first_escape) = bytes.find_byteset(escapes) else {
        return Ok(Cow::Borrowed(text));
    };
    let mut decoded = Vec::with_capacity(bytes.len());
    decoded.extend_from_slice(&bytes[..first_escape]);
    bytes = &bytes[first_escape..];
    while let Some((&byte, rest)) = bytes.split_first() {
        if byte == quote {
            decoded.push(quote);
            bytes = rest.strip_prefix(&[quote]).unwrap_or(rest);
        } else if byte == b'\\' {
            bytes = rest;
            decode_escape(&mut bytes, &mut decoded)?;
        } else {
            let end = bytes.find_byteset(escapes).unwrap_or(bytes.len());
            decoded.extend_from_slice(&bytes[..end]);
            bytes = &bytes[end..];
        }
    }
    String::from_utf8(decoded)
        .map(Cow::Owned)
        .map_err(|_| Error::NotImplemented("non-UTF-8 type-header name".into()))
}

/// Consumes the bytes after a backslash and appends the decoded ClickHouse escape.
fn decode_escape(bytes: &mut &[u8], decoded: &mut Vec<u8>) -> crate::Result<()> {
    let Some((&escaped, rest)) = bytes.split_first() else {
        return Err(Error::Parse("incomplete quoted escape".into()));
    };
    *bytes = rest;
    match escaped {
        b'x' => {
            let Some((hex, rest)) = bytes.split_first_chunk::<2>() else {
                return Err(Error::Parse("incomplete hexadecimal escape".into()));
            };
            let high = char::from(hex[0]).to_digit(16);
            let low = char::from(hex[1]).to_digit(16);
            let (Some(high), Some(low)) = (high, low) else {
                return Err(Error::Parse("invalid hexadecimal escape".into()));
            };
            decoded.push(u8::try_from(high * 16 + low).expect("two hex digits fit in u8"));
            *bytes = rest;
        }
        // `\N` is ClickHouse's NULL literal; inside a quoted name it decodes to nothing.
        b'N' => {}
        byte => {
            let value = match byte {
                b'a' => 0x07,
                b'b' => 0x08,
                b'e' => 0x1b,
                b'f' => 0x0c,
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'v' => 0x0b,
                b'0' => 0,
                byte => byte,
            };
            // Preserve unknown escapes for `LIKE` patterns such as `\%` and `\_`.
            // Quotes, `/` (JavaScript in HTML), `=` (TSKV) and controls are recognised.
            if !matches!(value, b'\\' | b'\'' | b'"' | b'`' | b'/' | b'=')
                && !value.is_ascii_control()
            {
                decoded.push(b'\\');
            }
            decoded.push(value);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::types::{Field, Type};
    use chrono_tz::Tz::UTC;
    #[test]
    fn decimal() {
        let input = b"Decimal(9, 9)";
        let typ = Type::from_bytes(input).unwrap();
        assert_eq!(typ, Type::Decimal32(9));
    }

    #[test]
    fn decimal_sized_aliases() {
        for (input, expected) in [
            (&b"Decimal32(3)"[..], Type::Decimal32(3)),
            (b"Decimal64(18)", Type::Decimal64(18)),
            (b"Decimal128(10)", Type::Decimal128(10)),
            (b"Decimal256(76)", Type::Decimal256(76)),
            (
                b"Nullable(Decimal32(0))",
                Type::Nullable(Box::new(Type::Decimal32(0))),
            ),
        ] {
            let typ = Type::from_bytes(input).unwrap();

            assert_eq!(typ, expected, "{}", String::from_utf8_lossy(input));
        }
    }

    #[test]
    fn decimal_sized_scale_exceeds_max_precision() {
        assert!(Type::from_bytes(b"Decimal32(10)").is_err());
        assert!(Type::from_bytes(b"Decimal64(19)").is_err());
        assert!(Type::from_bytes(b"Decimal128(39)").is_err());
        assert!(Type::from_bytes(b"Decimal256(77)").is_err());
        // `Decimal32(P, S)` is not a ClickHouse type: `Decimal32` takes the scale only.
        assert!(Type::from_bytes(b"Decimal32(9, 3)").is_err());
    }

    #[test]
    fn time_types() {
        for (input, expected) in [
            (&b"Time"[..], Type::Time),
            (b"Time64", Type::Time64(3)),
            (b"Time64(0)", Type::Time64(0)),
            (b"Time64(9)", Type::Time64(9)),
            (b"Array(Time)", Type::Array(Box::new(Type::Time))),
            (
                b"Nullable(Time64(6))",
                Type::Nullable(Box::new(Type::Time64(6))),
            ),
        ] {
            let typ = Type::from_bytes(input).unwrap();

            assert_eq!(typ, expected, "{}", String::from_utf8_lossy(input));
        }
    }

    #[test]
    fn geometry_types() {
        for (input, expected) in [
            (&b"MultiPoint"[..], Type::MultiPoint),
            (b"MultiPolygon", Type::MultiPolygon),
            (b"Geometry", Type::Geometry),
            (b"Array(Geometry)", Type::Array(Box::new(Type::Geometry))),
        ] {
            let typ = Type::from_bytes(input).unwrap();

            assert_eq!(typ, expected, "{}", String::from_utf8_lossy(input));
        }
    }

    #[test]
    fn decimal_scale_exceeds_precision() {
        assert!(Type::from_bytes(b"Decimal(9, 10)").is_err());
        assert!(Type::from_bytes(b"Decimal(18, 30)").is_err());
        assert!(Type::from_bytes(b"Decimal(38, 40)").is_err());
    }

    #[test]
    fn decimal_precision_out_of_range() {
        assert!(Type::from_bytes(b"Decimal(77, 0)").is_err());
    }

    #[test]
    fn int64() {
        let input = b"Int64";
        let result = Type::from_bytes(input);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Type::Int64);
    }

    #[test]
    fn map() {
        let input = b"Map(Int32, String)";
        let typ = Type::from_bytes(input).unwrap();
        assert_eq!(
            typ,
            Type::Map(Box::new(Type::Int32), Box::new(Type::String))
        );
    }

    #[test]
    fn map_nullable() {
        let input = b"Map(Int32, Nullable(LowCardinality(String)))";
        let typ = Type::from_bytes(input).unwrap();
        assert_eq!(
            typ,
            Type::Map(
                Box::new(Type::Int32),
                Box::new(Type::Nullable(Box::new(Type::LowCardinality(Box::new(
                    Type::String
                )))))
            )
        );
    }

    #[test]
    fn array() {
        let input = b"Array(Int32)";
        let typ = Type::from_bytes(input).unwrap();
        assert_eq!(typ, Type::Array(Box::new(Type::Int32)));
    }

    #[test]
    fn variant() {
        let input = b"Variant(Array(UInt64), String, UInt64)";
        let typ = Type::from_bytes(input).unwrap();
        assert_eq!(
            typ,
            Type::Variant(vec![
                Type::Array(Box::new(Type::UInt64)),
                Type::String,
                Type::UInt64
            ])
        );
    }

    #[test]
    fn dynamic_max_types() {
        for input in [
            &b"Dynamic"[..],
            b"Dynamic(max_types=0)",
            b"Dynamic(max_types=255)",
        ] {
            let typ = Type::from_bytes(input).unwrap();

            assert_eq!(typ, Type::Dynamic);
        }
    }

    #[test]
    fn array_nested() {
        let input = b"Array(Nested(child_id UInt64, child_name String, scores Array(UInt32)))";
        let typ = Type::from_bytes(input).unwrap();
        assert_eq!(
            typ,
            Type::Array(Box::new(Type::Nested(vec![
                Field {
                    name: "child_id".into(),
                    typ: Type::UInt64
                },
                Field {
                    name: "child_name".into(),
                    typ: Type::String
                },
                Field {
                    name: "scores".into(),
                    typ: Type::Array(Box::new(Type::UInt32))
                }
            ])))
        );
    }

    #[test]
    fn array_named_tuple() {
        let input = b"Array(Tuple(kind String, agent_symbols Bool, file_or_func_id UInt128, addr_or_line UInt64))";
        let typ = Type::from_bytes(input).unwrap();
        assert_eq!(
            typ,
            Type::Array(Box::new(Type::NamedTuple(vec![
                Field {
                    name: "kind".into(),
                    typ: Type::String
                },
                Field {
                    name: "agent_symbols".into(),
                    typ: Type::Bool
                },
                Field {
                    name: "file_or_func_id".into(),
                    typ: Type::UInt128
                },
                Field {
                    name: "addr_or_line".into(),
                    typ: Type::UInt64
                },
            ])))
        );
    }

    #[test]
    fn enum8() {
        let input = b"Enum8('Red' = 1, 'Green' = 2, 'Blue' = 3)";
        let typ = Type::from_bytes(input).unwrap();
        assert_eq!(
            typ,
            Type::Enum8(vec![
                ("Red".into(), 1),
                ("Green".into(), 2),
                ("Blue".into(), 3)
            ])
        );
    }

    #[test]
    fn enum16() {
        let input = b"Enum16('Foo' = 1000, 'Bar' = 2000)";
        let typ = Type::from_bytes(input).unwrap();
        assert_eq!(
            typ,
            Type::Enum16(vec![("Foo".into(), 1000), ("Bar".into(), 2000)])
        );
    }

    #[test]
    fn enum16_negative() {
        let input = b"Enum16('Min' = -32768, 'Neg' = -5000, 'Pos' = 5000)";
        let typ = Type::from_bytes(input).unwrap();
        assert_eq!(
            typ,
            Type::Enum16(vec![
                ("Min".into(), -32768),
                ("Neg".into(), -5000),
                ("Pos".into(), 5000)
            ])
        );
    }

    #[test]
    fn enum_rejects_unsorted_or_duplicate_ids() {
        assert!(Type::from_bytes(b"Enum8('B' = 2, 'A' = 1)").is_err());
        assert!(Type::from_bytes(b"Enum16('A' = 1, 'B' = 1)").is_err());
        assert!(Type::from_bytes(b"Enum8('Blue' = -23, 'Green' = 2, 'Red' = 11)").is_ok());
    }

    #[test]
    fn enum_empty_name() {
        let typ = Type::from_bytes(b"Enum8('' = 0, 'a' = 1)").unwrap();
        assert_eq!(typ, Type::Enum8(vec![("".into(), 0), ("a".into(), 1)]));
    }

    #[test]
    fn enum_label_whitespace_is_preserved() {
        for (input, expected) in [
            (
                &b"Enum8(' leading' = 1, 'trailing ' = 2, ' \t ' = 3)"[..],
                Type::Enum8(vec![
                    (" leading".into(), 1),
                    ("trailing ".into(), 2),
                    (" \t ".into(), 3),
                ]),
            ),
            (
                &b"Enum16(' leading' = 1, 'trailing ' = 2, ' \t ' = 3)"[..],
                Type::Enum16(vec![
                    (" leading".into(), 1),
                    ("trailing ".into(), 2),
                    (" \t ".into(), 3),
                ]),
            ),
        ] {
            let typ = Type::from_bytes(input).unwrap();

            assert_eq!(typ, expected);
        }
    }

    #[test]
    fn json_with_typed_paths_and_settings() {
        let typ = Type::from_bytes(
            b"JSON(max_dynamic_paths=2, `nested.name` String, a UInt64, \
              max_dynamic_types=4, SKIP ignored, SKIP REGEXP '^private')",
        )
        .unwrap();
        assert_eq!(
            typ,
            Type::Json(vec![
                Field {
                    name: "a".into(),
                    typ: Type::UInt64,
                },
                Field {
                    name: "nested.name".into(),
                    typ: Type::String,
                },
            ])
        );
    }

    #[test]
    fn json_type_rejects_unparsed_arguments() {
        assert!(Type::from_bytes(b"JSON(a UInt64) trailing").is_err());
        assert!(Type::from_bytes(b"JSON(unknown_setting=1)").is_err());
    }

    #[test]
    fn json_without_arguments() {
        assert_eq!(Type::from_bytes(b"JSON").unwrap(), Type::Json(vec![]));
        assert_eq!(Type::from_bytes(b"JSON()").unwrap(), Type::Json(vec![]));
    }

    #[test]
    fn named_tuple_identifier_may_start_with_underscore() {
        let typ = Type::from_bytes(b"Tuple(_id UInt64, name String)").unwrap();
        let Type::NamedTuple(fields) = typ else {
            panic!("expected NamedTuple, got {typ:?}");
        };
        let names: Vec<&str> = fields.iter().map(|f| f.name.as_ref()).collect();
        assert_eq!(names, ["_id", "name"]);
    }

    #[test]
    fn named_tuple_and_nested_backquoted_names() {
        let typ = Type::from_bytes(b"Tuple(`my field` UInt64, `1x` String, plain Int8)").unwrap();
        let Type::NamedTuple(fields) = typ else {
            panic!("expected NamedTuple, got {typ:?}");
        };
        let names: Vec<&str> = fields.iter().map(|f| f.name.as_ref()).collect();
        assert_eq!(names, ["my field", "1x", "plain"]);

        let typ = Type::from_bytes(b"Nested(`a.b` UInt64, c String)").unwrap();
        let Type::Nested(fields) = typ else {
            panic!("expected Nested, got {typ:?}");
        };
        let names: Vec<&str> = fields.iter().map(|f| f.name.as_ref()).collect();
        assert_eq!(names, ["a.b", "c"]);

        assert!(Type::from_bytes(b"Tuple(`unterminated UInt64)").is_err());
    }

    #[test]
    fn nothing() {
        assert_eq!(
            Type::from_bytes(b"Array(Nothing)").unwrap(),
            Type::Array(Box::new(Type::Nothing))
        );
        assert_eq!(
            Type::from_bytes(b"Nullable(Nothing)").unwrap(),
            Type::Nullable(Box::new(Type::Nothing))
        );
    }

    #[test]
    fn datetime_forms() {
        use chrono_tz::{Asia::Tokyo, Europe::Berlin};
        assert_eq!(Type::from_bytes(b"DateTime").unwrap(), Type::DateTime(UTC));
        assert_eq!(
            Type::from_bytes(b"DateTime('Europe/Berlin')").unwrap(),
            Type::DateTime(Berlin)
        );
        assert_eq!(
            Type::from_bytes(b"DateTime64").unwrap(),
            Type::DateTime64(3, UTC)
        );
        assert_eq!(
            Type::from_bytes(b"DateTime64(6)").unwrap(),
            Type::DateTime64(6, UTC)
        );
        assert_eq!(
            Type::from_bytes(b"DateTime64(9, 'Asia/Tokyo')").unwrap(),
            Type::DateTime64(9, Tokyo)
        );
        assert_eq!(
            Type::from_bytes(b"Nullable(DateTime64(3))").unwrap(),
            Type::Nullable(Box::new(Type::DateTime64(3, UTC)))
        );
        assert!(Type::from_bytes(b"DateTime('Mars/Olympus')").is_err());
        assert!(Type::from_bytes(b"DateTime64(3, 'Mars/Olympus')").is_err());
        assert!(Type::from_bytes(b"DateTime64('UTC')").is_err());
    }

    #[test]
    fn keywords_remain_names_without_confusing_positional_tuples() {
        assert_eq!(
            Type::from_bytes(b"Tuple(String, Array(UInt8), Tuple())").unwrap(),
            Type::Tuple(vec![
                Type::String,
                Type::Array(Box::new(Type::UInt8)),
                Type::Tuple(vec![]),
            ])
        );
        assert_eq!(
            Type::from_bytes(b"Tuple(String UInt8, Array String, SKIP Bool)").unwrap(),
            Type::NamedTuple(vec![
                Field {
                    name: "String".into(),
                    typ: Type::UInt8
                },
                Field {
                    name: "Array".into(),
                    typ: Type::String
                },
                Field {
                    name: "SKIP".into(),
                    typ: Type::Bool
                },
            ])
        );
        assert_eq!(
            Type::from_bytes(
                b"JSON(SKIP String, SKIP REGEXP '^secret', String UInt8, `SKIP` Bool)"
            )
            .unwrap(),
            Type::Json(vec![
                Field {
                    name: "SKIP".into(),
                    typ: Type::Bool
                },
                Field {
                    name: "String".into(),
                    typ: Type::UInt8
                },
            ])
        );
    }

    #[test]
    fn rejects_malformed_type_boundaries() {
        for input in [
            &b"StringSuffix"[..],
            b"Array(UInt8",
            b"Tuple(UInt8,)",
            b"Tuple(name UInt8, String)",
            b"Variant()",
            b"Nested()",
            b"Enum8()",
            b"Tuple(`name`UInt8)",
            b"JSON(`path`UInt8)",
            b"Time64(3, 'UTC')",
            b"FixedString(18446744073709551616)",
            b"DateTime64(256)",
            b"Enum8('overflow' = 128)",
            b"Enum16('underflow' = -32769)",
            // `IDataType::getName` writes whitespace only as `, `, one space between a
            // name and its type, ` = ` in enums and after `SKIP`/`REGEXP`.
            b"LowCardinality( String )",
            b"Map(String,UInt8)",
            b"Tuple(a  UInt8)",
            b"Tuple(a\tUInt8)",
            b"Enum8('a'=1)",
            b"Enum8('a' =1)",
            b"Dynamic(max_types = 5)",
            b"JSON(SKIP  a)",
            b" String",
            b"String ",
        ] {
            assert!(Type::from_bytes(input).is_err(), "{input:?}");
        }
    }

    #[test]
    fn borrows_utf8_names_and_rejects_invalid_utf8() {
        let input = String::from("Tuple(`café` String, UInt8 Enum8('日本語' = 1))");
        let Type::NamedTuple(fields) = Type::from_bytes(input.as_bytes()).unwrap() else {
            panic!("expected named tuple");
        };
        assert_eq!(fields[0].name, "café");
        assert_eq!(fields[0].name.as_ptr(), input[7..].as_ptr());
        let name_start = input.find("UInt8").unwrap();
        assert_eq!(fields[1].name.as_ptr(), input[name_start..].as_ptr());
        let Type::Enum8(labels) = &fields[1].typ else {
            panic!("expected enum");
        };
        assert_eq!(labels, &[("日本語".into(), 1)]);
        let label_start = input.find("日本語").unwrap();
        assert_eq!(labels[0].0.as_ptr(), input[label_start..].as_ptr());
        assert!(matches!(
            Type::from_bytes(b"Tuple(`\xff` UInt8)"),
            Err(crate::Error::Utf8Decode(..))
        ));
    }

    #[test]
    fn decodes_clickhouse_escapes_in_quoted_names() {
        let input = br"Tuple(`a\`b` Enum8('c\\d' = -2, 'a\'b' = 1, 'x''y' = 2, '\xE6\x97\xA5' = 3, '\%\_\N\n\0' = 4))";
        let typ = Type::from_bytes(input).unwrap();
        let Type::NamedTuple(fields) = typ else {
            panic!("expected named tuple");
        };
        assert_eq!(fields[0].name, "a`b");
        let Type::Enum8(labels) = &fields[0].typ else {
            panic!("expected enum");
        };
        let labels: Vec<&str> = labels.iter().map(|(label, _)| label.as_ref()).collect();
        assert_eq!(labels, ["c\\d", "a'b", "x'y", "日", "\\%\\_\n\0"]);

        for input in [
            &br"Enum8('a' = 1, '\x61' = 2)"[..],
            br"Enum8('\x0' = 1)",
            br"Enum8('unterminated\' = 1)",
            br"Tuple(`unterminated\` UInt8)",
            br"Tuple(`a`UInt8)",
        ] {
            assert!(
                matches!(Type::from_bytes(input), Err(crate::Error::Parse(_))),
                "{}",
                String::from_utf8_lossy(input)
            );
        }
        assert!(matches!(
            Type::from_bytes(br"Enum8('\xFF' = 1)"),
            Err(crate::Error::NotImplemented(_))
        ));
    }

    #[test]
    fn escaped_names_preserve_long_utf8_segments() {
        let text = "日本語é".repeat(32);
        let input =
            format!("Tuple(`{text}\\n{text}` Enum8('{text}''\\xE6\\x97\\xA5{text}\\N' = 0))");
        let typ = Type::from_bytes(input.as_bytes()).unwrap();
        assert_eq!(
            typ,
            Type::NamedTuple(vec![Field {
                name: format!("{text}\n{text}").into(),
                typ: Type::Enum8(vec![(format!("{text}'日{text}").into(), 0)]),
            }])
        );
    }

    #[test]
    fn aggregate_function_and_qbit_are_not_implemented_at_any_depth() {
        for input in [
            &b"AggregateFunction(count)"[..],
            b"Array(AggregateFunction(sum, UInt64))",
            b"Tuple(x String, state AggregateFunction(1, quantiles(0.5, 0.9), UInt64))",
            b"Map(String, AggregateFunction(sum, UInt64))",
            b"JSON(state AggregateFunction(sum, UInt64))",
            br"AggregateFunction(f((1, 2), tuple('a\'b'), [1, 2], 0.5::Float64), UInt64)",
            b"Array(Nullable(QBit(Float32, 3)))",
            b"Tuple(QBit(Int8, 16, 8))",
            b"QBit(BFloat16, 8)",
        ] {
            assert!(
                matches!(
                    Type::from_bytes(input),
                    Err(crate::Error::NotImplemented(_))
                ),
                "{}",
                String::from_utf8_lossy(input)
            );
        }
        for input in [
            &b"AggregateFunction(sum, UInt64"[..],
            b"AggregateFunction(1,)",
            b"AggregateFunction(f((1, 2), UInt64)",
            b"QBit(String, 3)",
            b"QBit(Float32, 0)",
            b"QBit(Float32, 16, 3)",
            b"QBit(Float32, 16, 32)",
        ] {
            assert!(
                matches!(Type::from_bytes(input), Err(crate::Error::Parse(_))),
                "{}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn simple_aggregate_function_is_its_storage_type() {
        assert_eq!(
            Type::from_bytes(b"SimpleAggregateFunction(sum, UInt64)").unwrap(),
            Type::UInt64
        );
        assert_eq!(
            Type::from_bytes(b"SimpleAggregateFunction(anyLast, Nullable(String))").unwrap(),
            Type::Nullable(Box::new(Type::String))
        );
        assert_eq!(
            Type::from_bytes(b"SimpleAggregateFunction(groupArrayArray(3), Array(UInt64))")
                .unwrap(),
            Type::Array(Box::new(Type::UInt64))
        );
        assert_eq!(
            Type::from_bytes(b"SimpleAggregateFunction(sumMap, Map(String, UInt64))").unwrap(),
            Type::Map(Box::new(Type::String), Box::new(Type::UInt64))
        );
        assert!(Type::from_bytes(b"SimpleAggregateFunction(sum)").is_err());
        assert!(Type::from_bytes(b"SimpleAggregateFunction('sum', UInt64)").is_err());
    }

    #[test]
    fn nesting_depth_is_bounded_like_clickhouse() {
        use crate::parse::consts::MAX_TYPE_DEPTH;

        let nested = |depth: usize| {
            let mut s = "Array(".repeat(depth);
            s.push_str("UInt8");
            s.push_str(&")".repeat(depth));
            s
        };
        assert!(Type::from_bytes(nested(MAX_TYPE_DEPTH).as_bytes()).is_ok());
        assert!(Type::from_bytes(nested(MAX_TYPE_DEPTH + 1).as_bytes()).is_err());
        // Parameter parentheses count too: the limit is total nesting, not type nesting.
        let params = format!(
            "SimpleAggregateFunction(f{}{}, UInt8)",
            "(".repeat(MAX_TYPE_DEPTH),
            ")".repeat(MAX_TYPE_DEPTH)
        );
        assert!(Type::from_bytes(params.as_bytes()).is_err());
        // Deep enough to overflow the stack in the recursive consumers without the guard.
        assert!(Type::from_bytes(nested(100_000).as_bytes()).is_err());
    }

    // The keyword list is spelled out in `lexer::keywords!`, the grammar's `extern` block
    // and `OrdinaryWord`. A keyword missing from the latter two surfaces here.
    #[test]
    fn every_keyword_is_a_valid_field_name() {
        for (_, keyword) in super::lexer::Kw::ALL {
            let header = format!("Tuple({keyword} UInt8)");
            let result = Type::from_bytes(header.as_bytes());
            let Ok(Type::NamedTuple(fields)) = result else {
                panic!("{header}: {result:?}");
            };
            assert_eq!(fields[0].name, *keyword);
        }
    }
}
