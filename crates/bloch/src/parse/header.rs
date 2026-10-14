use std::{borrow::Cow, hint::cold_path};

use log::debug;

use crate::{
    Error,
    parse::{
        IResult,
        block::ParseContext,
        column::string,
        consts::{
            DYNAMIC_SERIALIZATION_V1, DYNAMIC_SERIALIZATION_V2, JSON_SERIALIZATION_V1,
            JSON_SERIALIZATION_V2, LOW_CARDINALITY_VERSION, MAX_DYNAMIC_TYPES,
        },
        parse_u64, parse_var_str, parse_varuint,
    },
    types::{DynamicHeader, Field, JsonColumnHeader, JsonHeader, MapHeader, Type, TypeHeader},
};

pub fn variant<'a>(
    ctx: &ParseContext<'a>,
    inner: &[Type<'a>],
) -> IResult<&'a [u8], Vec<TypeHeader<'a>>> {
    let (input, mode) = parse_u64::<u64>(ctx.input)?;
    if mode != 0 {
        cold_path();
        return Err(Error::Parse(format!(
            "Variant mode {mode} is not supported, only 0 is allowed"
        )));
    }
    many(&ctx.fork(input), inner.iter())
}

// Reads what `SerializationDynamic::serializeBinaryBulkStatePrefix` writes.
pub fn dynamic<'a>(ctx: &ParseContext<'a>) -> IResult<&'a [u8], DynamicHeader<'a>> {
    let (mut input, version) = parse_u64::<u64>(ctx.input)?;
    match version {
        DYNAMIC_SERIALIZATION_V1 => {
            let legacy_max_types: u64;
            (input, legacy_max_types) = parse_varuint(input)?;
            debug!("Legacy max_dynamic_types: {legacy_max_types}");
        }
        DYNAMIC_SERIALIZATION_V2 => {}
        other => {
            cold_path();
            return Err(Error::NotImplemented(format!(
                "Dynamic serialization version {other}"
            )));
        }
    }

    let (mut input, num_types) = parse_varuint::<usize>(input)?;
    if num_types > MAX_DYNAMIC_TYPES {
        cold_path();
        return Err(Error::CorruptedData(format!(
            "Dynamic column has too many types: {num_types} (max {MAX_DYNAMIC_TYPES})"
        )));
    }
    let mut type_names = Vec::with_capacity(num_types + 1);
    for _ in 0..num_types {
        let t;
        (input, t) = parse_var_str(input)?;
        type_names.push(t);
    }
    type_names.push("SharedVariant");
    // https://github.com/ClickHouse/clickhouse-go/blob/a27396fbf07ca38de1d452c5b366b3a37ce45f56/lib/column/dynamic.go#L366
    type_names.sort_unstable();

    debug!("Dynamic type names (sorted): {type_names:?}");

    let mut types = Vec::with_capacity(num_types + 1);
    for name in type_names {
        types.push(Type::from_bytes(name.as_bytes())?);
    }

    debug!("Dynamic types: {types:?}");

    // No statistics precede the Variant prefix: `NativeWriter` leaves `write_statistics` at
    // `StatisticsMode::NONE`.
    let headers;
    (input, headers) = variant(&ctx.fork(input), &types)?;

    Ok((input, DynamicHeader { types, headers }))
}

pub fn map<'a>(
    ctx: &ParseContext<'a>,
    key: &Type<'a>,
    val: &Type<'a>,
) -> IResult<&'a [u8], MapHeader<'a>> {
    let (input, key_th) = key.decode_header(ctx)?;
    let (input, val_th) = val.decode_header(&ctx.fork(input))?;
    let h = MapHeader {
        key: key_th,
        value: val_th,
    };

    Ok((input, h))
}

pub fn nested<'a>(
    ctx: &ParseContext<'a>,
    fields: &[Field<'a>],
) -> IResult<&'a [u8], Vec<TypeHeader<'a>>> {
    many(ctx, fields.iter().map(|f| &f.typ))
}

pub fn named_tuple<'a>(
    ctx: &ParseContext<'a>,
    fields: &[Field<'a>],
) -> IResult<&'a [u8], Vec<TypeHeader<'a>>> {
    many(ctx, fields.iter().map(|f| &f.typ))
}

pub fn point<'a>() -> TypeHeader<'a> {
    TypeHeader::Tuple(vec![TypeHeader::Empty, TypeHeader::Empty])
}

pub fn ring<'a>() -> TypeHeader<'a> {
    TypeHeader::Array(Box::new(point()))
}

pub fn polygon<'a>() -> TypeHeader<'a> {
    TypeHeader::Array(Box::new(ring()))
}

pub fn multi_polygon<'a>() -> TypeHeader<'a> {
    TypeHeader::Array(Box::new(polygon()))
}

pub fn tuple<'a>(
    ctx: &ParseContext<'a>,
    inner: &[Type<'a>],
) -> IResult<&'a [u8], Vec<TypeHeader<'a>>> {
    many(ctx, inner.iter())
}

pub fn lc<'a>(ctx: &ParseContext<'a>) -> IResult<&'a [u8], TypeHeader<'a>> {
    let (input, version) = parse_u64::<u64>(ctx.input)?;
    debug!("LowCardinality version: {version}");
    if version == LOW_CARDINALITY_VERSION {
        return Ok((input, TypeHeader::Empty));
    }

    Err({
        cold_path();
        Error::Parse(format!(
            "LowCardinality version {version} is not supported, only {LOW_CARDINALITY_VERSION} is \
             allowed"
        ))
    })
}

// Reads what `SerializationObject::serializeBinaryBulkStatePrefix` writes for V1/V2, in order:
//   1. structure: version, [V1: max dynamic paths], num dynamic paths, dynamic path names;
//   2. the prefix of every typed path, in name order;
//   3. for every dynamic path, in name order, a complete `Dynamic` prefix
//      (`SerializationDynamic::serializeBinaryBulkStatePrefix`, which in turn nests the
//      `Variant` prefix: mode + every variant's prefix);
//   4. the shared data prefix.
pub fn json<'a>(
    ctx: &ParseContext<'a>,
    typed_paths: &[Field<'a>],
) -> IResult<&'a [u8], JsonHeader<'a>> {
    debug_assert!(
        typed_paths.is_sorted_by(|left, right| left.name <= right.name),
        "typed path prefixes are written in name order"
    );
    let (mut input, version) = parse_u64::<u64>(ctx.input)?;
    debug!("JSON version: {version}");
    match version {
        JSON_SERIALIZATION_V1 => {
            let max_dynamic_paths: u64;
            (input, max_dynamic_paths) = parse_varuint(input)?;
            debug!("JSON max dynamic paths: {max_dynamic_paths}");
        }
        JSON_SERIALIZATION_V2 => {}
        other => {
            cold_path();
            return Err(Error::NotImplemented(format!(
                "JSON serialization version {other}"
            )));
        }
    }

    let (input, num_dynamic_paths) = parse_varuint(input)?;
    let (mut input, dynamic_paths) = string(&ctx.fork(input).with_num_rows(num_dynamic_paths))?;

    let cap = typed_paths.len() + num_dynamic_paths.min(input.len());
    let mut paths = Vec::with_capacity(cap);
    let mut col_headers = Vec::with_capacity(cap);
    for field in typed_paths {
        paths.push(field.name.clone());
    }
    for path in dynamic_paths.data {
        paths.push(Cow::Borrowed(crate::error::decode_utf8(path)?));
    }

    for field in typed_paths {
        let header;
        (input, header) = field.typ.decode_header(&ctx.fork(input))?;
        col_headers.push(JsonColumnHeader::Typed {
            typ: field.typ.clone(),
            header,
        });
    }
    for _ in 0..num_dynamic_paths {
        let header;
        (input, header) = dynamic(&ctx.fork(input))?;
        col_headers.push(JsonColumnHeader::Dynamic(header));
    }
    // The shared data prefix is that of `Map(String, String)`, which is empty.

    Ok((input, JsonHeader { paths, col_headers }))
}

fn many<'a, 'b>(
    ctx: &ParseContext<'a>,
    types: impl Iterator<Item = &'b Type<'a>>,
) -> IResult<&'a [u8], Vec<TypeHeader<'a>>>
where
    'a: 'b,
{
    let mut headers = Vec::new();
    let mut ctx = ctx.clone();
    for typ in types {
        let (input, th) = typ.decode_header(&ctx)?;
        headers.push(th);
        ctx = ctx.fork(input);
    }
    Ok((ctx.input, headers))
}
