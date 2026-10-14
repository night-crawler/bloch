#[cfg(feature = "serde1")]
use std::borrow::Cow;
use std::{hint::cold_path, ops::Range};

use super::{Readable, TryRead};
#[cfg(feature = "serde1")]
use crate::{ByteExt as _, types::OffsetIndexPair as _};
use crate::{Error, mark, value::Value};

#[cfg(feature = "serde1")]
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};

#[derive(Clone, Copy)]
pub struct Json<'a>(&'a mark::Json<'a>);

impl<'a> TryFrom<&'a mark::Mark<'a>> for Json<'a> {
    type Error = Error;

    fn try_from(value: &'a mark::Mark<'a>) -> Result<Self, Self::Error> {
        match value {
            mark::Mark::Json(mark) => Ok(Self(mark)),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), Self::NAME))
            }
        }
    }
}

impl<'a> TryRead<'a> for Json<'a> {
    type Item = JsonValue<'a>;

    const NAME: &'static str = "Json";

    fn len(&self) -> usize {
        self.0.len()
    }

    unsafe fn try_read_unchecked(&self, row: usize) -> crate::Result<Self::Item> {
        Ok(JsonValue { mark: self.0, row })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct JsonValue<'a> {
    mark: &'a mark::Json<'a>,
    row: usize,
}

/// How JSON path segments are exposed as object keys during serde deserialization.
///
/// Native data does not carry `json_type_escape_dots_in_keys`. Select the mode that
/// matches the producer's setting; literal `%2E` and an escaped dot are indistinguishable.
#[cfg(feature = "serde1")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum JsonKeyMode {
    /// Decode ClickHouse's uppercase `%2E` escape after splitting dotted paths.
    /// Lowercase `%2e` and all other percent sequences remain literal.
    #[default]
    Decode,
    /// Preserve every path segment verbatim, including `%2E`.
    Preserve,
}

/// Options shared by a JSON deserialization and all of its nested values.
#[cfg(feature = "serde1")]
#[derive(Clone, Debug, Default)]
pub struct DeserializeConfig {
    /// How JSON path segments are exposed as object keys. Defaults to decoding.
    pub key_mode: JsonKeyMode,
}

impl<'a> JsonValue<'a> {
    /// Iterate raw, flattened Native paths, without decoding escaped dots.
    pub const fn paths(self) -> JsonIterator<'a> {
        JsonIterator {
            mark: self.mark,
            row: self.row,
            path_index: 0,
        }
    }

    /// Deserialize with [`DeserializeConfig::default`], including nested JSON values.
    ///
    /// Use [`Self::deserialize_with_config`] to override the defaults.
    #[cfg(feature = "serde1")]
    pub fn deserialize<T>(self) -> Result<T, JsonDeserializeError>
    where
        T: serde::Deserialize<'a>,
    {
        self.deserialize_with_config(&DeserializeConfig::default())
    }

    /// Deserialize with a shared configuration, applied recursively to JSON objects.
    ///
    /// Ordinary string values, map keys, and named-tuple fields are not decoded.
    /// Both modes support borrowed keys. Decoded keys are cached in the parsed block.
    #[cfg(feature = "serde1")]
    pub fn deserialize_with_config<T>(
        self,
        config: &DeserializeConfig,
    ) -> Result<T, JsonDeserializeError>
    where
        T: serde::Deserialize<'a>,
    {
        let states = build_node_states(self.mark, self.row, config)?;
        T::deserialize(NodeDeserializer {
            mark: self.mark,
            row: self.row,
            node: self.mark.root(),
            config,
            states: &states,
        })
    }
}

impl<'a> Readable<'a> for JsonValue<'a> {
    type Reader = Json<'a>;
}

impl<'a> TryFrom<Value<'a>> for JsonValue<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::Json { mark, index } => Ok(Self { mark, row: index }),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), "JsonValue"))
            }
        }
    }
}

pub struct JsonIterator<'a> {
    mark: &'a mark::Json<'a>,
    row: usize,
    path_index: usize,
}

impl<'a> TryFrom<Value<'a>> for JsonIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match JsonValue::try_from(value) {
            Ok(value) => Ok(value.paths()),
            Err(error) => Err(error),
        }
    }
}

impl<'a> Iterator for JsonIterator<'a> {
    type Item = Result<(&'a str, Value<'a>), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let path_index = self.path_index;
            if path_index >= self.mark.paths.len() {
                return None;
            }
            let path = self.mark.paths[path_index].as_ref();
            self.path_index += 1;

            match self.mark.value(path_index, self.row) {
                Ok(Some(value)) => break Some(Ok((path, value))),
                Ok(None) => continue,
                Err(error) => break Some(Err(error)),
            }
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.mark.paths.len() - self.path_index;
        (0, Some(remaining))
    }
}

pub struct JsonSliceIterator<'a> {
    mark: &'a mark::Json<'a>,
    range: Range<usize>,
}

impl<'a> TryFrom<Value<'a>> for JsonSliceIterator<'a> {
    type Error = Error;

    fn try_from(value: Value<'a>) -> Result<Self, Self::Error> {
        match value {
            Value::JsonSlice { mark, range } => Ok(Self {
                mark,
                range: range.into(),
            }),
            other => {
                cold_path();
                Err(Error::MismatchedType(other.as_str(), "JsonSliceIterator"))
            }
        }
    }
}

impl<'a> Iterator for JsonSliceIterator<'a> {
    type Item = JsonIterator<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let row = self.range.next()?;
        Some(
            JsonValue {
                mark: self.mark,
                row,
            }
            .paths(),
        )
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.range.size_hint()
    }
}

impl ExactSizeIterator for JsonSliceIterator<'_> {}

#[cfg(feature = "serde1")]
#[derive(Debug, thiserror::Error)]
pub enum JsonDeserializeError {
    #[error(transparent)]
    Source(Box<Error>),
    #[error("conflicting scalar and object JSON path at {0:?}")]
    StructuralConflict(String),
    #[error("unsupported ClickHouse JSON value type: {0}")]
    Unsupported(&'static str),
    #[error("{0}")]
    Message(String),
}

#[cfg(feature = "serde1")]
impl From<Error> for JsonDeserializeError {
    fn from(error: Error) -> Self {
        Self::Source(Box::new(error))
    }
}

#[cfg(feature = "serde1")]
impl de::Error for JsonDeserializeError {
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        Self::Message(message.to_string())
    }
}

#[cfg(feature = "serde1")]
#[derive(Clone, Copy)]
struct NodeDeserializer<'de, 'config> {
    mark: &'de mark::Json<'de>,
    row: usize,
    node: usize,
    config: &'config DeserializeConfig,
    states: &'config [NodeState],
}

#[cfg(feature = "serde1")]
#[derive(Clone, Copy)]
enum NodeShape<'de> {
    Leaf(Cell<'de>),
    Object,
}

#[cfg(feature = "serde1")]
#[derive(Clone, Copy, PartialEq, Eq)]
enum NodeState {
    Absent,
    Leaf,
    Object,
    Conflict,
}

// Resolves row-dependent presence once, bottom-up, so the entire serde walk shares it
// instead of every ancestor rescanning each subtree from scratch.
#[cfg(feature = "serde1")]
fn build_node_states(
    mark: &mark::Json<'_>,
    row: usize,
    config: &DeserializeConfig,
) -> Result<Box<[NodeState]>, JsonDeserializeError> {
    let mut states = vec![NodeState::Absent; mark.node_count()].into_boxed_slice();

    // Node indices increase from parent to child (established in `PathTree::child`),
    // so this single reverse pass resolves every child before its parent needs it.
    for node in (0..states.len()).rev() {
        let leaf = if let Some(path) = mark.node_leaf(node)
            && let Some(column) = mark.columns.get(path)
            && !mark.is_absent(path, row)
        {
            CellDeserializer {
                mark: column,
                row,
                config,
            }
            .is_present()?
        } else {
            false
        };
        let mut child = mark.first_child(node);
        let mut has_child = false;
        while let Some(index) = child {
            debug_assert!(
                index > node,
                "PathTree child index must exceed its parent's"
            );
            if states[index] != NodeState::Absent {
                has_child = true;
                break;
            }
            child = mark.next_sibling(index);
        }
        states[node] = match (leaf, has_child) {
            (false, false) => NodeState::Absent,
            (true, false) => NodeState::Leaf,
            (false, true) => NodeState::Object,
            (true, true) => NodeState::Conflict,
        };
    }
    Ok(states)
}

#[cfg(feature = "serde1")]
impl<'de, 'config> NodeDeserializer<'de, 'config> {
    fn shape(self) -> Result<NodeShape<'de>, JsonDeserializeError> {
        match self.states[self.node] {
            NodeState::Leaf => {
                let path = self
                    .mark
                    .node_leaf(self.node)
                    .expect("active leaf has a path");
                Ok(NodeShape::Leaf(Cell {
                    mark: &self.mark.columns[path],
                    row: self.row,
                }))
            }
            NodeState::Conflict => {
                let path = self
                    .mark
                    .node_leaf(self.node)
                    .expect("conflicting leaf has a path");
                Err(JsonDeserializeError::StructuralConflict(
                    self.mark.paths[path].to_string(),
                ))
            }
            NodeState::Absent | NodeState::Object => Ok(NodeShape::Object),
        }
    }

    fn leaf(self) -> Result<CellDeserializer<'de, 'config>, JsonDeserializeError> {
        match self.shape()? {
            NodeShape::Leaf(cell) => Ok(cell.deserializer(self.config)),
            NodeShape::Object => Err(JsonDeserializeError::Unsupported("JSON object enum")),
        }
    }
}

#[cfg(feature = "serde1")]
impl<'de> de::Deserializer<'de> for NodeDeserializer<'de, '_> {
    type Error = JsonDeserializeError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.shape()? {
            NodeShape::Leaf(cell) => cell.deserializer(self.config).deserialize_any(visitor),
            NodeShape::Object => visitor.visit_map(PathMapAccess {
                mark: self.mark,
                row: self.row,
                next_child: self.mark.first_child(self.node),
                pending: None,
                config: self.config,
                states: self.states,
            }),
        }
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.leaf()?.deserialize_bytes(visitor)
    }

    fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.leaf()?.deserialize_byte_buf(visitor)
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.shape()? {
            NodeShape::Leaf(cell) => cell.deserializer(self.config).deserialize_option(visitor),
            NodeShape::Object => visitor.visit_some(self),
        }
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.leaf()?.deserialize_enum(name, variants, visitor)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        unit unit_struct seq tuple tuple_struct map struct identifier ignored_any
    }

    fn is_human_readable(&self) -> bool {
        true
    }
}

#[cfg(feature = "serde1")]
struct PathMapAccess<'de, 'config> {
    mark: &'de mark::Json<'de>,
    row: usize,
    next_child: Option<usize>,
    pending: Option<usize>,
    config: &'config DeserializeConfig,
    states: &'config [NodeState],
}

#[cfg(feature = "serde1")]
impl<'de> MapAccess<'de> for PathMapAccess<'de, '_> {
    type Error = JsonDeserializeError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: DeserializeSeed<'de>,
    {
        while let Some(index) = self.next_child {
            self.next_child = self.mark.next_sibling(index);
            if self.states[index] == NodeState::Absent {
                continue;
            }
            self.pending = Some(index);
            let key: &'de str = self
                .mark
                .node_key(index, self.config.key_mode == JsonKeyMode::Decode);
            return match seed.deserialize(de::value::BorrowedStrDeserializer::new(key)) {
                Ok(key) => Ok(Some(key)),
                Err(error) => Err(error),
            };
        }
        Ok(None)
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        let Some(node) = self.pending.take() else {
            return Err(JsonDeserializeError::Message(
                "JSON map value requested before its key".to_owned(),
            ));
        };
        seed.deserialize(NodeDeserializer {
            mark: self.mark,
            row: self.row,
            node,
            config: self.config,
            states: self.states,
        })
    }
}

// ClickHouse FORMAT JSON emits 256-bit values as unquoted JSON numbers. Serde's numeric model
// stops at 128 bits, so preserve their exact decimal representation as strings instead of
// forcing every consumer to enable serde_json's `arbitrary_precision` feature.
#[cfg(feature = "serde1")]
struct WideNumber {
    bytes: [u8; 80],
    len: usize,
}

#[cfg(feature = "serde1")]
impl WideNumber {
    const fn push(&mut self, byte: u8) {
        self.bytes[self.len] = byte;
        self.len += 1;
    }

    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).expect("wide number formatter emits ASCII")
    }
}

#[cfg(feature = "serde1")]
fn format_wide_number(mut magnitude: [u8; 32], signed: bool, scale: u8) -> WideNumber {
    let negative = signed && magnitude[31] & 0x80 != 0;
    if negative {
        let mut carry = true;
        for byte in &mut magnitude {
            *byte = !*byte;
            if carry {
                let (value, overflow) = byte.overflowing_add(1);
                *byte = value;
                carry = overflow;
            }
        }
    }

    let mut formatted = WideNumber {
        bytes: [0; 80],
        len: 0,
    };
    let mut is_zero = true;
    for byte in &magnitude {
        if *byte != 0 {
            is_zero = false;
            break;
        }
    }
    if is_zero {
        formatted.push(b'0');
        return formatted;
    }

    let mut reversed = [0_u8; 78];
    let mut digit_count = 0;
    loop {
        let mut remainder = 0_u16;
        let mut has_more = false;
        for byte in magnitude.iter_mut().rev() {
            let dividend = (remainder << 8) | u16::from(*byte);
            *byte = u8::try_from(dividend / 10).expect("quotient fits in u8");
            remainder = dividend % 10;
            has_more |= *byte != 0;
        }
        reversed[digit_count] = b'0' + u8::try_from(remainder).expect("remainder is below 10");
        digit_count += 1;
        if !has_more {
            break;
        }
    }

    let mut trimmed = 0;
    let scale = usize::from(scale);
    while trimmed < scale && reversed[trimmed] == b'0' {
        trimmed += 1;
    }
    let scale = scale - trimmed;
    let digits = digit_count - trimmed;
    if negative {
        formatted.push(b'-');
    }
    if scale == 0 {
        for index in (trimmed..digit_count).rev() {
            formatted.push(reversed[index]);
        }
    } else if digits > scale {
        for index in ((trimmed + scale)..digit_count).rev() {
            formatted.push(reversed[index]);
        }
        formatted.push(b'.');
        for index in (trimmed..(trimmed + scale)).rev() {
            formatted.push(reversed[index]);
        }
    } else {
        formatted.push(b'0');
        formatted.push(b'.');
        for _ in 0..(scale - digits) {
            formatted.push(b'0');
        }
        for index in (trimmed..digit_count).rev() {
            formatted.push(reversed[index]);
        }
    }
    formatted
}

#[cfg(feature = "serde1")]
#[derive(Clone, Copy)]
struct Cell<'de> {
    mark: &'de mark::Mark<'de>,
    row: usize,
}

#[cfg(feature = "serde1")]
impl<'de> Cell<'de> {
    const fn deserializer<'config>(
        self,
        config: &'config DeserializeConfig,
    ) -> CellDeserializer<'de, 'config> {
        CellDeserializer {
            mark: self.mark,
            row: self.row,
            config,
        }
    }
}

#[cfg(feature = "serde1")]
#[derive(Clone, Copy)]
struct CellDeserializer<'de, 'config> {
    mark: &'de mark::Mark<'de>,
    row: usize,
    config: &'config DeserializeConfig,
}

#[cfg(feature = "serde1")]
#[derive(Clone, Copy)]
enum CellState<'de> {
    Missing,
    Null,
    Present(Cell<'de>),
}

#[cfg(feature = "serde1")]
impl<'de> CellDeserializer<'de, '_> {
    fn state(self) -> Result<CellState<'de>, JsonDeserializeError> {
        let mut cell = Cell {
            mark: self.mark,
            row: self.row,
        };
        loop {
            match cell.mark {
                mark::Mark::Empty => return Ok(CellState::Missing),
                mark::Mark::Nullable(nullable) => match nullable.is_null(cell.row) {
                    Some(true) => return Ok(CellState::Null),
                    Some(false) => {
                        cell.mark = &nullable.data;
                    }
                    None => return Ok(CellState::Missing),
                },
                mark::Mark::LowCardinality(low_cardinality) => {
                    let Some(index) = low_cardinality.indices.get(cell.row)? else {
                        return Ok(CellState::Missing);
                    };
                    if low_cardinality.is_null(index) {
                        return Ok(CellState::Null);
                    }
                    let keys = low_cardinality.keys()?;
                    cell = Cell {
                        mark: keys,
                        row: index,
                    };
                }
                mark::Mark::Variant(variant) => {
                    let Some(&discriminator) = variant.discriminators.get(cell.row) else {
                        return Ok(CellState::Missing);
                    };
                    if discriminator == mark::Variant::NULL_DISCRIMINATOR {
                        return Ok(CellState::Null);
                    }
                    let Some(&row) = variant.offsets.get(cell.row) else {
                        return Ok(CellState::Missing);
                    };
                    let Some(mark) = variant.types.get(usize::from(discriminator)) else {
                        return Ok(CellState::Missing);
                    };
                    cell = Cell {
                        mark,
                        row: row as usize,
                    };
                }
                mark::Mark::Dynamic(dynamic) => {
                    let Some(&discriminator) = dynamic.discriminators.get(cell.row) else {
                        return Ok(CellState::Missing);
                    };
                    if discriminator == mark::Variant::NULL_DISCRIMINATOR {
                        return Ok(CellState::Null);
                    }
                    let Some(&row) = dynamic.offsets.get(cell.row) else {
                        return Ok(CellState::Missing);
                    };
                    let Some(mark) = dynamic.columns.get(usize::from(discriminator)) else {
                        return Ok(CellState::Missing);
                    };
                    cell = Cell {
                        mark,
                        row: row as usize,
                    };
                }
                mark => {
                    return if cell.row < mark.len() {
                        Ok(CellState::Present(cell))
                    } else {
                        Ok(CellState::Missing)
                    };
                }
            }
        }
    }

    fn is_present(self) -> Result<bool, JsonDeserializeError> {
        Ok(!matches!(self.state()?, CellState::Missing))
    }

    fn borrowed_str(self) -> Result<Option<&'de str>, JsonDeserializeError> {
        let CellState::Present(cell) = self.state()? else {
            return Ok(None);
        };
        let value = match cell.mark {
            mark::Mark::String(strings) => strings
                .get(cell.row)
                .map(|value| crate::error::decode_utf8(value))
                .transpose()?,
            mark::Mark::FixedString(fixed) => fixed
                .get_bstr(cell.row)
                .map(|value| crate::error::decode_utf8(value.rtrim_zeros()))
                .transpose()?,
            mark::Mark::Enum8(enumeration) => enumeration.name(cell.row),
            mark::Mark::Enum16(enumeration) => enumeration.name(cell.row),
            _ => None,
        };
        Ok(value)
    }

    fn borrowed_bytes(self) -> Result<Option<&'de [u8]>, JsonDeserializeError> {
        let CellState::Present(cell) = self.state()? else {
            return Ok(None);
        };
        Ok(match cell.mark {
            mark::Mark::String(strings) => strings.get(cell.row).map(AsRef::as_ref),
            mark::Mark::FixedString(fixed) => fixed.get_bstr(cell.row).map(AsRef::as_ref),
            mark::Mark::Enum8(enumeration) => enumeration.name(cell.row).map(str::as_bytes),
            mark::Mark::Enum16(enumeration) => enumeration.name(cell.row).map(str::as_bytes),
            _ => None,
        })
    }
}

#[cfg(feature = "serde1")]
impl<'de> de::Deserializer<'de> for CellDeserializer<'de, '_> {
    type Error = JsonDeserializeError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let cell = match self.state()? {
            CellState::Missing => {
                return Err(Error::IndexOutOfBounds(self.row, self.mark.as_str()).into());
            }
            CellState::Null => return visitor.visit_unit(),
            CellState::Present(cell) => cell,
        };

        macro_rules! at {
            ($value:expr) => {{
                let Some(value) = $value else {
                    return Err(Error::IndexOutOfBounds(cell.row, cell.mark.as_str()).into());
                };
                value
            }};
        }

        match cell.mark {
            mark::Mark::Empty => unreachable!("empty marks are resolved as missing"),
            mark::Mark::Nothing(len) => {
                if cell.row >= *len {
                    return Err(Error::IndexOutOfBounds(cell.row, "Nothing").into());
                }
                visitor.visit_unit()
            }
            mark::Mark::Bool(value) => visitor.visit_bool(at!(value.get(cell.row))),
            mark::Mark::Int8(value) => visitor.visit_i8(*at!(value.get(cell.row))),
            mark::Mark::Int16(value) => visitor.visit_i16(at!(value.get(cell.row)).get()),
            mark::Mark::Int32(value) | mark::Mark::Time(value) => {
                visitor.visit_i32(at!(value.get(cell.row)).get())
            }
            mark::Mark::Int64(value) => visitor.visit_i64(at!(value.get(cell.row)).get()),
            mark::Mark::Int128(value) => visitor.visit_i128(at!(value.get(cell.row)).get()),
            mark::Mark::Int256(value) => {
                let formatted = format_wide_number(at!(value.get(cell.row)).0, true, 0);
                visitor.visit_str(formatted.as_str())
            }
            mark::Mark::UInt8(value) => visitor.visit_u8(*at!(value.get(cell.row))),
            mark::Mark::UInt16(value) => visitor.visit_u16(at!(value.get(cell.row)).get()),
            mark::Mark::UInt32(value) => visitor.visit_u32(at!(value.get(cell.row)).get()),
            mark::Mark::UInt64(value) => visitor.visit_u64(at!(value.get(cell.row)).get()),
            mark::Mark::UInt128(value) => visitor.visit_u128(at!(value.get(cell.row)).get()),
            mark::Mark::UInt256(value) => {
                let formatted = format_wide_number(at!(value.get(cell.row)).0, false, 0);
                visitor.visit_str(formatted.as_str())
            }
            mark::Mark::Float32(value) => visitor.visit_f32(at!(value.get(cell.row)).get()),
            mark::Mark::Float64(value) => visitor.visit_f64(at!(value.get(cell.row)).get()),
            mark::Mark::BFloat16(value) => {
                let value = half::bf16::from(*at!(value.get(cell.row)));
                visitor.visit_f32(value.into())
            }
            mark::Mark::Decimal32(value) => visitor.visit_string(
                at!(value.data.get(cell.row))
                    .with_scale(value.scale)
                    .to_string(),
            ),
            mark::Mark::Decimal64(value) => visitor.visit_string(
                at!(value.data.get(cell.row))
                    .with_scale(value.scale)
                    .to_string(),
            ),
            mark::Mark::Decimal128(value) => visitor.visit_string(
                at!(value.data.get(cell.row))
                    .with_scale(value.scale)?
                    .to_string(),
            ),
            mark::Mark::Decimal256(value) => {
                let formatted =
                    format_wide_number(at!(value.data.get(cell.row)).0.0, true, value.scale);
                visitor.visit_str(formatted.as_str())
            }
            mark::Mark::String(value) => {
                let value = at!(value.get(cell.row));
                match crate::error::decode_utf8(value) {
                    Ok(value) => visitor.visit_borrowed_str(value),
                    Err(_) => visitor.visit_borrowed_bytes(value),
                }
            }
            mark::Mark::FixedString(value) => {
                let value = at!(value.get_bstr(cell.row));
                match crate::error::decode_utf8(value.rtrim_zeros()) {
                    Ok(value) => visitor.visit_borrowed_str(value),
                    Err(_) => visitor.visit_borrowed_bytes(value),
                }
            }
            mark::Mark::Uuid(value) => {
                let value = uuid::Uuid::from(*at!(value.get(cell.row)));
                visitor.visit_string(value.to_string())
            }
            mark::Mark::Date(value) => {
                let value = chrono::NaiveDate::from(*at!(value.get(cell.row)));
                visitor.visit_string(value.to_string())
            }
            mark::Mark::Date32(value) => {
                let value = chrono::NaiveDate::try_from(*at!(value.get(cell.row)))?;
                visitor.visit_string(value.to_string())
            }
            mark::Mark::DateTime(value) => {
                let value = at!(value.data.get(cell.row)).with_tz(value.tz);
                visitor.visit_string(value.to_rfc3339())
            }
            mark::Mark::DateTime64(value) => {
                let value = at!(value.data.get(cell.row))
                    .with_tz_and_precision(value.tz, value.precision)?;
                visitor.visit_string(value.to_rfc3339())
            }
            mark::Mark::Time64(value) => visitor.visit_i64(at!(value.data.get(cell.row)).get()),
            mark::Mark::Interval(value) => visitor.visit_i64(at!(value.data.get(cell.row)).get()),
            mark::Mark::Ipv4(value) => {
                let value = std::net::Ipv4Addr::from(*at!(value.get(cell.row)));
                visitor.visit_string(value.to_string())
            }
            mark::Mark::Ipv6(value) => {
                let value = std::net::Ipv6Addr::from(*at!(value.get(cell.row)));
                visitor.visit_string(value.to_string())
            }
            mark::Mark::Enum8(_) | mark::Mark::Enum16(_) => {
                let Some(value) = self.borrowed_str()? else {
                    return Err(Error::IndexOutOfBounds(cell.row, cell.mark.as_str()).into());
                };
                visitor.visit_borrowed_str(value)
            }
            mark::Mark::Array(array) => {
                let Some((start, end)) = array.offsets.offset_indices(cell.row)? else {
                    return Err(Error::IndexOutOfBounds(cell.row, "Array").into());
                };
                visitor.visit_seq(ColumnSeqAccess {
                    mark: &array.values,
                    range: start..end,
                    config: self.config,
                })
            }
            mark::Mark::Tuple(tuple) => visitor.visit_seq(TupleSeqAccess {
                tuple,
                row: cell.row,
                next: 0,
                config: self.config,
            }),
            mark::Mark::Map(map) => {
                visitor.visit_map(ColumnMapAccess::new(map, cell.row, self.config)?)
            }
            mark::Mark::Nested(nested) => {
                let mark::Mark::Array(array) = nested.array_of_tuples.as_ref() else {
                    return Err(
                        Error::MismatchedType(nested.array_of_tuples.as_str(), "Array").into(),
                    );
                };
                let mark::Mark::Tuple(tuple) = array.values.as_ref() else {
                    return Err(Error::MismatchedType(array.values.as_str(), "Tuple").into());
                };
                let Some((start, end)) = array.offsets.offset_indices(cell.row)? else {
                    return Err(Error::IndexOutOfBounds(cell.row, "Nested").into());
                };
                visitor.visit_seq(NamedRowsSeqAccess {
                    names: &nested.col_names,
                    tuple,
                    range: start..end,
                    config: self.config,
                })
            }
            mark::Mark::NamedTuple(named) => {
                let mark::Mark::Tuple(tuple) = named.tuple.as_ref() else {
                    return Err(Error::MismatchedType(named.tuple.as_str(), "Tuple").into());
                };
                visitor.visit_map(NamedMapAccess {
                    names: &named.col_names,
                    tuple,
                    row: cell.row,
                    next: 0,
                    pending: None,
                    config: self.config,
                })
            }
            mark::Mark::Json(json) => {
                let states = build_node_states(json, cell.row, self.config)?;
                NodeDeserializer {
                    mark: json,
                    row: cell.row,
                    node: json.root(),
                    config: self.config,
                    states: &states,
                }
                .deserialize_any(visitor)
            }
            mark::Mark::Nullable(_)
            | mark::Mark::LowCardinality(_)
            | mark::Mark::Variant(_)
            | mark::Mark::Dynamic(_) => {
                unreachable!("wrapper marks are resolved before deserialization")
            }
        }
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.borrowed_bytes()? {
            Some(value) => visitor.visit_borrowed_bytes(value),
            None => self.deserialize_any(visitor),
        }
    }

    fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.borrowed_bytes()? {
            Some(value) => visitor.visit_byte_buf(value.to_vec()),
            None => self.deserialize_any(visitor),
        }
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.state()? {
            CellState::Missing => Err(Error::IndexOutOfBounds(self.row, self.mark.as_str()).into()),
            CellState::Null => visitor.visit_none(),
            CellState::Present(cell) => visitor.visit_some(cell.deserializer(self.config)),
        }
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let Some(value) = self.borrowed_str()? else {
            return Err(JsonDeserializeError::Message(format!(
                "expected enum string, got {}",
                self.mark.as_str()
            )));
        };
        visitor.visit_enum(de::value::BorrowedStrDeserializer::<JsonDeserializeError>::new(value))
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        unit unit_struct seq tuple tuple_struct map struct identifier ignored_any
    }

    fn is_human_readable(&self) -> bool {
        true
    }
}

#[cfg(feature = "serde1")]
struct ColumnSeqAccess<'de, 'config> {
    mark: &'de mark::Mark<'de>,
    range: Range<usize>,
    config: &'config DeserializeConfig,
}

#[cfg(feature = "serde1")]
impl<'de> SeqAccess<'de> for ColumnSeqAccess<'de, '_> {
    type Error = JsonDeserializeError;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
    where
        T: DeserializeSeed<'de>,
    {
        let Some(row) = self.range.next() else {
            return Ok(None);
        };
        match seed.deserialize(CellDeserializer {
            mark: self.mark,
            row,
            config: self.config,
        }) {
            Ok(value) => Ok(Some(value)),
            Err(error) => Err(error),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.range.len())
    }
}

#[cfg(feature = "serde1")]
struct TupleSeqAccess<'de, 'config> {
    tuple: &'de mark::Tuple<'de>,
    row: usize,
    next: usize,
    config: &'config DeserializeConfig,
}

#[cfg(feature = "serde1")]
impl<'de> SeqAccess<'de> for TupleSeqAccess<'de, '_> {
    type Error = JsonDeserializeError;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
    where
        T: DeserializeSeed<'de>,
    {
        let Some(mark) = self.tuple.values.get(self.next) else {
            return Ok(None);
        };
        self.next += 1;
        match seed.deserialize(CellDeserializer {
            mark,
            row: self.row,
            config: self.config,
        }) {
            Ok(value) => Ok(Some(value)),
            Err(error) => Err(error),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.tuple.values.len() - self.next)
    }
}

#[cfg(feature = "serde1")]
struct ColumnMapAccess<'de, 'config> {
    keys: &'de mark::Mark<'de>,
    values: &'de mark::Mark<'de>,
    range: Range<usize>,
    pending: Option<usize>,
    config: &'config DeserializeConfig,
}

#[cfg(feature = "serde1")]
impl<'de, 'config> ColumnMapAccess<'de, 'config> {
    fn new(
        mark: &'de mark::Map<'de>,
        row: usize,
        config: &'config DeserializeConfig,
    ) -> Result<Self, JsonDeserializeError> {
        let Some((start, end)) = mark.offsets.offset_indices(row)? else {
            return Err(Error::IndexOutOfBounds(row, "Map").into());
        };
        Ok(Self {
            keys: &mark.keys,
            values: &mark.values,
            range: start..end,
            pending: None,
            config,
        })
    }
}

#[cfg(feature = "serde1")]
impl<'de> MapAccess<'de> for ColumnMapAccess<'de, '_> {
    type Error = JsonDeserializeError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: DeserializeSeed<'de>,
    {
        let Some(row) = self.range.next() else {
            return Ok(None);
        };
        self.pending = Some(row);
        match seed.deserialize(CellDeserializer {
            mark: self.keys,
            row,
            config: self.config,
        }) {
            Ok(value) => Ok(Some(value)),
            Err(error) => Err(error),
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        let Some(row) = self.pending.take() else {
            return Err(JsonDeserializeError::Message(
                "map value requested before its key".to_owned(),
            ));
        };
        seed.deserialize(CellDeserializer {
            mark: self.values,
            row,
            config: self.config,
        })
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.range.len())
    }
}

#[cfg(feature = "serde1")]
struct NamedMapAccess<'de, 'config> {
    names: &'de [Cow<'de, str>],
    tuple: &'de mark::Tuple<'de>,
    row: usize,
    next: usize,
    pending: Option<usize>,
    config: &'config DeserializeConfig,
}

#[cfg(feature = "serde1")]
impl<'de> MapAccess<'de> for NamedMapAccess<'de, '_> {
    type Error = JsonDeserializeError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: DeserializeSeed<'de>,
    {
        let Some(name) = self.names.get(self.next) else {
            return Ok(None);
        };
        if self.tuple.values.get(self.next).is_none() {
            return Err(JsonDeserializeError::Message(
                "named tuple has fewer values than names".to_owned(),
            ));
        }
        self.pending = Some(self.next);
        self.next += 1;
        match seed.deserialize(de::value::BorrowedStrDeserializer::new(name.as_ref())) {
            Ok(name) => Ok(Some(name)),
            Err(error) => Err(error),
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        let Some(index) = self.pending.take() else {
            return Err(JsonDeserializeError::Message(
                "tuple value requested before its key".to_owned(),
            ));
        };
        seed.deserialize(CellDeserializer {
            mark: &self.tuple.values[index],
            row: self.row,
            config: self.config,
        })
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.names.len() - self.next)
    }
}

#[cfg(feature = "serde1")]
struct NamedRowsSeqAccess<'de, 'config> {
    names: &'de [Cow<'de, str>],
    tuple: &'de mark::Tuple<'de>,
    range: Range<usize>,
    config: &'config DeserializeConfig,
}

#[cfg(feature = "serde1")]
impl<'de> SeqAccess<'de> for NamedRowsSeqAccess<'de, '_> {
    type Error = JsonDeserializeError;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, Self::Error>
    where
        T: DeserializeSeed<'de>,
    {
        let Some(row) = self.range.next() else {
            return Ok(None);
        };
        match seed.deserialize(NamedRowDeserializer {
            names: self.names,
            tuple: self.tuple,
            row,
            config: self.config,
        }) {
            Ok(value) => Ok(Some(value)),
            Err(error) => Err(error),
        }
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.range.len())
    }
}

#[cfg(feature = "serde1")]
#[derive(Clone, Copy)]
struct NamedRowDeserializer<'de, 'config> {
    names: &'de [Cow<'de, str>],
    tuple: &'de mark::Tuple<'de>,
    row: usize,
    config: &'config DeserializeConfig,
}

#[cfg(feature = "serde1")]
impl<'de> de::Deserializer<'de> for NamedRowDeserializer<'de, '_> {
    type Error = JsonDeserializeError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_map(NamedMapAccess {
            names: self.names,
            tuple: self.tuple,
            row: self.row,
            next: 0,
            pending: None,
            config: self.config,
        })
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_some(self)
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_newtype_struct(self)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
        bytes byte_buf unit unit_struct seq tuple tuple_struct map struct enum identifier ignored_any
    }

    fn is_human_readable(&self) -> bool {
        true
    }
}

#[cfg(all(test, feature = "serde1"))]
mod serde_tests {
    use bstr::BStr;
    use std::{collections::BTreeMap, net::Ipv4Addr};

    use serde::Deserialize;
    use serde_json::json;
    use testresult::TestResult;

    use super::{DeserializeConfig, Json, JsonDeserializeError, JsonKeyMode, format_wide_number};
    use crate::{
        DateTime32Data, Ipv4Data, mark, parse::block::parse_single, reader::TryRead as _,
        slice::ByteView,
    };

    fn string_view(data: Vec<&str>) -> mark::StringView<'_> {
        mark::StringView {
            data: data.into_iter().map(BStr::new).collect(),
        }
    }

    #[test]
    fn deserializes_flat_nested_and_borrowed_structs() -> TestResult {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Flat<'a> {
            #[serde(borrow)]
            key: &'a str,
        }
        #[derive(Debug, Deserialize, PartialEq)]
        struct Nested {
            a: u64,
            b: u64,
        }
        #[derive(Debug, Deserialize, PartialEq)]
        struct Root {
            nested: Nested,
        }

        let data = crate::common::load("./testdata/json.native")?;
        let (_, block) = parse_single(&data)?;
        let reader = Json::try_from(block.mark("json")?)?;

        let flat: Flat<'_> = reader.try_read(0)?.deserialize()?;
        assert_eq!(flat, Flat { key: "value" });
        let input = data.as_ptr_range();
        assert!(input.contains(&flat.key.as_ptr()));

        let nested: Root = reader.try_read(2)?.deserialize()?;
        assert_eq!(
            nested,
            Root {
                nested: Nested { a: 1, b: 2 }
            }
        );
        Ok(())
    }

    #[test]
    fn handles_missing_null_empty_and_row_shape_changes() -> TestResult {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Missing<'a> {
            #[serde(borrow)]
            absent: Option<&'a str>,
        }
        #[derive(Debug, Deserialize, PartialEq)]
        struct Numbers {
            array: Vec<u64>,
        }
        #[derive(Debug, Deserialize, PartialEq)]
        struct Object {
            array: Child,
        }
        #[derive(Debug, Deserialize, PartialEq)]
        struct Child {
            haha: bool,
        }

        let data = crate::common::load("./testdata/json.native")?;
        let (_, block) = parse_single(&data)?;
        let reader = Json::try_from(block.mark("json")?)?;

        assert_eq!(
            reader.try_read(0)?.deserialize::<Missing<'_>>()?,
            Missing { absent: None }
        );
        assert_eq!(
            reader.try_read(4)?.deserialize::<serde_json::Value>()?,
            json!({})
        );
        assert_eq!(
            reader.try_read(9)?.deserialize::<serde_json::Value>()?,
            json!({})
        );
        assert_eq!(
            reader.try_read(1)?.deserialize::<Numbers>()?,
            Numbers {
                array: vec![1, 2, 3]
            }
        );
        assert_eq!(
            reader.try_read(7)?.deserialize::<Object>()?,
            Object {
                array: Child { haha: true }
            }
        );
        assert_eq!(
            reader.try_read(11)?.deserialize::<serde_json::Value>()?,
            json!({"mixed_types": ["1", "string", "true", null]})
        );
        Ok(())
    }

    #[test]
    fn supports_serde_field_attributes() -> TestResult {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Renamed<'a> {
            #[serde(borrow, rename = "key")]
            name: &'a str,
        }
        #[derive(Debug, Deserialize, PartialEq)]
        struct Flattened<'a> {
            #[serde(borrow, flatten)]
            rest: BTreeMap<&'a str, &'a str>,
        }
        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Strict {}
        #[derive(Debug, Deserialize, PartialEq)]
        struct Lenient {}

        let data = crate::common::load("./testdata/json.native")?;
        let (_, block) = parse_single(&data)?;
        let reader = Json::try_from(block.mark("json")?)?;

        assert_eq!(
            reader.try_read(0)?.deserialize::<Renamed<'_>>()?,
            Renamed { name: "value" }
        );
        assert_eq!(
            reader.try_read(0)?.deserialize::<Flattened<'_>>()?,
            Flattened {
                rest: BTreeMap::from([("key", "value")])
            }
        );
        assert_eq!(reader.try_read(0)?.deserialize::<Lenient>()?, Lenient {});
        assert!(reader.try_read(0)?.deserialize::<Strict>().is_err());
        Ok(())
    }

    #[test]
    fn rejects_active_conflicting_paths() -> TestResult {
        let mark = mark::Mark::Json(mark::Json::new(
            vec!["a".into(), "a.b".into()],
            vec![
                mark::Mark::String(string_view(vec!["x"])),
                mark::Mark::String(string_view(vec!["y"])),
            ],
            2,
            1,
        )?);
        let error = Json::try_from(&mark)?
            .try_read(0)?
            .deserialize::<serde_json::Value>()
            .expect_err("active scalar and child paths must conflict");
        assert!(matches!(error, JsonDeserializeError::StructuralConflict(path) if path == "a"));
        Ok(())
    }

    #[test]
    fn deserializes_large_sparse_tree_across_rows() -> TestResult {
        let paths: Vec<_> = (0..80).map(|i| format!("root.branch{i}.leaf")).collect();
        let columns = paths
            .iter()
            .enumerate()
            .map(|(i, path)| {
                mark::Mark::Dynamic(mark::Dynamic {
                    offsets: vec![0; 3].into_boxed_slice(),
                    discriminators: if i % 2 == 0 {
                        &[0, 255, 255]
                    } else {
                        &[255, 0, 255]
                    },
                    columns: vec![mark::Mark::String(string_view(vec![path.as_str()]))]
                        .into_boxed_slice(),
                })
            })
            .collect();
        let mark = mark::Mark::Json(mark::Json::new(
            paths.iter().map(|path| path.as_str().into()).collect(),
            columns,
            0,
            3,
        )?);
        let reader = Json::try_from(&mark)?;
        for row in [0, 1, 2, 0] {
            let mut expected = serde_json::Map::new();
            for (i, path) in paths.iter().enumerate() {
                if i % 2 == row {
                    expected.insert(format!("branch{i}"), json!({"leaf": path}));
                }
            }
            let expected = if row == 2 {
                json!({})
            } else {
                json!({"root": expected})
            };
            assert_eq!(
                reader.try_read(row)?.deserialize::<serde_json::Value>()?,
                expected,
                "row {row}",
            );
        }
        Ok(())
    }

    #[test]
    fn deserializes_nested_arrays_of_json_rows() -> TestResult {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Child<'a> {
            #[serde(borrow)]
            name: &'a str,
        }
        #[derive(Debug, Deserialize, PartialEq)]
        struct Root<'a> {
            #[serde(borrow)]
            items: Vec<Vec<Child<'a>>>,
        }

        let nested_json = mark::Json::new(
            vec!["name".into()],
            vec![mark::Mark::String(string_view(vec!["one", "two", "three"]))],
            1,
            3,
        )?;
        let inner_offsets = [1_u64.to_le_bytes(), 3_u64.to_le_bytes()].concat();
        let inner = mark::Mark::Array(mark::Array {
            offsets: ByteView::try_from(inner_offsets.as_slice())?,
            values: Box::new(mark::Mark::Json(nested_json)),
        });
        let outer_offsets = 2_u64.to_le_bytes();
        let outer = mark::Mark::Array(mark::Array {
            offsets: ByteView::try_from(outer_offsets.as_slice())?,
            values: Box::new(inner),
        });
        let mark = mark::Mark::Json(mark::Json::new(vec!["items".into()], vec![outer], 1, 1)?);

        let actual: Root<'_> = Json::try_from(&mark)?.try_read(0)?.deserialize()?;
        assert_eq!(
            actual,
            Root {
                items: vec![
                    vec![Child { name: "one" }],
                    vec![Child { name: "two" }, Child { name: "three" }]
                ]
            }
        );
        Ok(())
    }

    #[test]
    fn preserves_numeric_boundaries_and_scalar_string_policy() -> TestResult {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Boundaries {
            signed: i64,
            unsigned: u64,
        }

        let signed = i64::MIN.to_le_bytes();
        let unsigned = u64::MAX.to_le_bytes();
        let mark = mark::Mark::Json(mark::Json::new(
            vec!["signed".into(), "unsigned".into()],
            vec![
                mark::Mark::Int64(ByteView::try_from(signed.as_slice())?),
                mark::Mark::UInt64(ByteView::try_from(unsigned.as_slice())?),
            ],
            2,
            1,
        )?);
        assert_eq!(
            Json::try_from(&mark)?
                .try_read(0)?
                .deserialize::<Boundaries>()?,
            Boundaries {
                signed: i64::MIN,
                unsigned: u64::MAX
            }
        );

        let date = chrono::NaiveDate::from_ymd_opt(2024, 2, 29).unwrap();
        let epoch = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
        let date_days = u16::try_from((date - epoch).num_days())?.to_le_bytes();
        let address = Ipv4Addr::new(127, 0, 0, 1);
        let address_bytes = u32::from(address).to_le_bytes();
        let uuid = uuid::Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef")?;
        let (hi, lo) = uuid.as_u64_pair();
        let uuid_bytes = [hi.to_le_bytes(), lo.to_le_bytes()].concat();
        let decimal_bytes = 12_345_i64.to_le_bytes();
        let datetime_bytes = 0_u32.to_le_bytes();
        let formatted = mark::Mark::Json(mark::Json::new(
            vec![
                "date".into(),
                "ip".into(),
                "uuid".into(),
                "decimal".into(),
                "datetime".into(),
            ],
            vec![
                mark::Mark::Date(ByteView::try_from(date_days.as_slice())?),
                mark::Mark::Ipv4(ByteView::<Ipv4Data>::try_from(address_bytes.as_slice())?),
                mark::Mark::Uuid(ByteView::try_from(uuid_bytes.as_slice())?),
                mark::Mark::Decimal64(mark::Decimal64 {
                    scale: 2,
                    data: ByteView::try_from(decimal_bytes.as_slice())?,
                }),
                mark::Mark::DateTime(mark::DateTime {
                    tz: chrono_tz::UTC,
                    data: ByteView::<DateTime32Data>::try_from(datetime_bytes.as_slice())?,
                }),
            ],
            5,
            1,
        )?);
        let actual: serde_json::Value = Json::try_from(&formatted)?.try_read(0)?.deserialize()?;
        assert_eq!(
            actual,
            json!({
                "date": "2024-02-29",
                "ip": "127.0.0.1",
                "uuid": uuid.to_string(),
                "decimal": "123.45",
                "datetime": "1970-01-01T00:00:00+00:00",
            })
        );
        Ok(())
    }

    #[test]
    fn preserves_wide_numeric_values_as_strings() -> TestResult {
        const I256_MIN: &str =
            "-57896044618658097711785492504343953926634992332820282019728792003956564819968";
        const U256_MAX: &str =
            "115792089237316195423570985008687907853269984665640564039457584007913129639935";

        let mut i256_min = [0_u8; 32];
        i256_min[31] = 0x80;
        let u256_max = [0xff_u8; 32];
        let mut decimal = [0xff_u8; 32];
        decimal[..16].copy_from_slice(&(-123_456_789_i128).to_le_bytes());
        let mut trailing_zeros = [0_u8; 32];
        trailing_zeros[..16].copy_from_slice(&12_000_u128.to_le_bytes());
        let minus_one = [0xff_u8; 32];

        assert_eq!(format_wide_number(i256_min, true, 0).as_str(), I256_MIN);
        assert_eq!(format_wide_number(u256_max, false, 0).as_str(), U256_MAX);
        assert_eq!(format_wide_number(decimal, true, 4).as_str(), "-12345.6789");
        assert_eq!(format_wide_number(trailing_zeros, false, 3).as_str(), "12");
        assert_eq!(format_wide_number(minus_one, true, 3).as_str(), "-0.001");
        assert_eq!(format_wide_number([0_u8; 32], true, 76).as_str(), "0");

        let mark = mark::Mark::Json(mark::Json::new(
            vec!["i".into(), "u".into(), "d".into()],
            vec![
                mark::Mark::Int256(ByteView::try_from(i256_min.as_slice())?),
                mark::Mark::UInt256(ByteView::try_from(u256_max.as_slice())?),
                mark::Mark::Decimal256(mark::Decimal256 {
                    scale: 4,
                    data: ByteView::try_from(decimal.as_slice())?,
                }),
            ],
            3,
            1,
        )?);
        let actual: serde_json::Value = Json::try_from(&mark)?.try_read(0)?.deserialize()?;
        let expected = json!({
            "i": I256_MIN,
            "u": U256_MAX,
            "d": "-12345.6789",
        });
        assert_eq!(actual, expected);

        let array_offsets = 1_u64.to_le_bytes();
        let array_mark = mark::Mark::Json(mark::Json::new(
            vec!["i_array".into(), "u_array".into(), "d_array".into()],
            vec![
                mark::Mark::Array(mark::Array {
                    offsets: ByteView::try_from(array_offsets.as_slice())?,
                    values: Box::new(mark::Mark::Int256(ByteView::try_from(i256_min.as_slice())?)),
                }),
                mark::Mark::Array(mark::Array {
                    offsets: ByteView::try_from(array_offsets.as_slice())?,
                    values: Box::new(mark::Mark::UInt256(ByteView::try_from(
                        u256_max.as_slice(),
                    )?)),
                }),
                mark::Mark::Array(mark::Array {
                    offsets: ByteView::try_from(array_offsets.as_slice())?,
                    values: Box::new(mark::Mark::Decimal256(mark::Decimal256 {
                        scale: 4,
                        data: ByteView::try_from(decimal.as_slice())?,
                    })),
                }),
            ],
            3,
            1,
        )?);
        let arrays: serde_json::Value = Json::try_from(&array_mark)?.try_read(0)?.deserialize()?;
        assert_eq!(arrays["i_array"], json!([I256_MIN]));
        assert_eq!(arrays["u_array"], json!([U256_MAX]));
        assert_eq!(arrays["d_array"], json!(["-12345.6789"]));
        Ok(())
    }

    #[test]
    fn deserializes_composite_marks() -> TestResult {
        let mask_present = [0_u8];
        let mask_null = [1_u8];
        let low_cardinality_indices = [0_u8];
        let variant_discriminators = [0_u8];
        let map_offsets = 1_u64.to_le_bytes();
        let nested_offsets = 2_u64.to_le_bytes();
        let map_value = 7_u64.to_le_bytes();
        let tuple_value = 1_u64.to_le_bytes();
        let named_value = 9_u64.to_le_bytes();

        let marks = vec![
            mark::Mark::Nullable(mark::Nullable {
                mask: &mask_present,
                data: Box::new(mark::Mark::String(string_view(vec!["some"]))),
            }),
            mark::Mark::Nullable(mark::Nullable {
                mask: &mask_null,
                data: Box::new(mark::Mark::String(string_view(vec!["unused"]))),
            }),
            mark::Mark::LowCardinality(mark::lc::LowCardinality {
                is_nullable: false,
                indices: mark::lc::Indices::U8(&low_cardinality_indices),
                global_dictionary: None,
                additional_keys: Some(Box::new(mark::Mark::String(string_view(vec!["dict"])))),
            }),
            mark::Mark::Map(mark::Map {
                offsets: ByteView::try_from(map_offsets.as_slice())?,
                keys: Box::new(mark::Mark::String(string_view(vec!["k"]))),
                values: Box::new(mark::Mark::UInt64(ByteView::try_from(
                    map_value.as_slice(),
                )?)),
            }),
            mark::Mark::Tuple(mark::Tuple {
                values: vec![
                    mark::Mark::UInt64(ByteView::try_from(tuple_value.as_slice())?),
                    mark::Mark::String(string_view(vec!["two"])),
                ]
                .into(),
                num_rows: 1,
            }),
            mark::Mark::NamedTuple(mark::NamedTuple {
                col_names: vec!["x".into()].into(),
                tuple: Box::new(mark::Mark::Tuple(mark::Tuple {
                    values: vec![mark::Mark::UInt64(ByteView::try_from(
                        named_value.as_slice(),
                    )?)]
                    .into(),
                    num_rows: 1,
                })),
            }),
            mark::Mark::Nested(mark::Nested {
                col_names: vec!["name".into()].into(),
                array_of_tuples: Box::new(mark::Mark::Array(mark::Array {
                    offsets: ByteView::try_from(nested_offsets.as_slice())?,
                    values: Box::new(mark::Mark::Tuple(mark::Tuple {
                        values: vec![mark::Mark::String(string_view(vec!["a", "b"]))].into(),
                        num_rows: 2,
                    })),
                })),
            }),
            mark::Mark::Variant(mark::Variant {
                offsets: vec![0].into(),
                discriminators: &variant_discriminators,
                types: vec![mark::Mark::String(string_view(vec!["variant"]))].into(),
            }),
            mark::Mark::Dynamic(mark::Dynamic {
                offsets: vec![0].into(),
                discriminators: &variant_discriminators,
                columns: vec![mark::Mark::String(string_view(vec!["dynamic"]))].into(),
            }),
        ];
        let mark = mark::Mark::Json(mark::Json::new(
            vec![
                "nullable".into(),
                "null".into(),
                "lc".into(),
                "map".into(),
                "tuple".into(),
                "named".into(),
                "nested".into(),
                "variant".into(),
                "dynamic".into(),
            ],
            marks,
            9,
            1,
        )?);

        let actual: serde_json::Value = Json::try_from(&mark)?.try_read(0)?.deserialize()?;
        assert_eq!(
            actual,
            json!({
                "nullable": "some",
                "null": null,
                "lc": "dict",
                "map": {"k": 7},
                "tuple": [1, "two"],
                "named": {"x": 9},
                "nested": [{"name": "a"}, {"name": "b"}],
                "variant": "variant",
                "dynamic": "dynamic",
            })
        );
        Ok(())
    }

    #[test]
    fn deserializes_typed_fixture_and_owned_value() -> TestResult {
        #[derive(Debug, Deserialize, PartialEq)]
        struct Nested<'a> {
            #[serde(borrow)]
            name: &'a str,
        }
        #[derive(Debug, Deserialize, PartialEq)]
        struct Typed<'a> {
            a: u64,
            #[serde(borrow)]
            nested: Nested<'a>,
            extra: Option<serde_json::Value>,
        }

        let data = crate::common::load("./testdata/json_typed.native")?;
        let (_, block) = parse_single(&data)?;
        let reader = Json::try_from(block.mark("json")?)?;
        assert_eq!(
            reader.try_read(0)?.deserialize::<Typed<'_>>()?,
            Typed {
                a: 42,
                nested: Nested { name: "alpha" },
                extra: Some(json!(true))
            }
        );
        assert_eq!(
            reader.try_read(2)?.deserialize::<serde_json::Value>()?,
            json!({"a": 0, "nested": {"name": ""}})
        );

        let shared = crate::common::load("./testdata/json_shared.native")?;
        let Err(error) = parse_single(&shared) else {
            panic!("shared data must remain rejected");
        };
        assert!(matches!(
            error,
            crate::Error::NotImplemented(message) if message == "non-empty JSON shared data"
        ));
        Ok(())
    }

    // ClickHouse keeps a typed path present even when its value is NULL (only dynamic-path
    // NULLs mean "absent"), so `Nullable(T)` typed paths must land in `Option<T>` as `None`
    // instead of failing on the unit value.
    #[test]
    fn deserializes_typed_nullable_path_into_option() -> TestResult {
        const _SQL: &str = r#"
drop table if exists json_typed_nullable;

create table json_typed_nullable
(
    id   UInt64,
    json JSON(k Nullable(Int64), n Nullable(String))
) engine = MergeTree order by id;

insert into json_typed_nullable (id, json) values
    (0, '{"k": 1, "n": "x", "free": true}'),
    (1, '{"k": null, "n": null}'),
    (2, '{"k": -7, "n": "", "free": "s"}');

select * from json_typed_nullable order by id format Native;
"#;

        #[derive(Debug, Deserialize, PartialEq)]
        struct Row<'a> {
            #[serde(borrow)]
            n: Option<&'a str>,
            k: Option<i64>,
            free: Option<serde_json::Value>,
        }

        let data = crate::common::load("./testdata/json_typed_nullable.native")?;
        let (_, block) = parse_single(&data)?;
        let reader = Json::try_from(block.mark("json")?)?;
        assert_eq!(
            reader.try_read(0)?.deserialize::<Row<'_>>()?,
            Row {
                n: Some("x"),
                k: Some(1),
                free: Some(json!(true))
            }
        );
        assert_eq!(
            reader.try_read(1)?.deserialize::<Row<'_>>()?,
            Row {
                n: None,
                k: None,
                free: None
            }
        );
        assert_eq!(
            reader.try_read(2)?.deserialize::<Row<'_>>()?,
            Row {
                n: Some(""),
                k: Some(-7),
                free: Some(json!("s"))
            }
        );
        assert_eq!(
            reader.try_read(1)?.deserialize::<serde_json::Value>()?,
            json!({"n": null, "k": null})
        );
        Ok(())
    }

    // ClickHouse 26.8: `SELECT ...::JSON(v Variant(Int64, String), n Nullable(String))` prints
    // `{"n":null,"v":null}` for both `{"v": null}` and `{}`: a typed `Variant` path is present as
    // NULL exactly like a typed `Nullable` path, and must not be dropped as a missing key.
    #[test]
    fn typed_variant_path_null_is_present() -> TestResult {
        const _SQL: &str = r#"
drop table if exists json_typed_variant;

create table json_typed_variant
(
    json JSON(n Nullable(String), v Variant(Int64, String))
) engine = MergeTree order by tuple();

insert into json_typed_variant (json) values
    ('{}'),
    ('{"n": null, "v": null}'),
    ('{"n": "s", "v": 1}'),
    ('{"n": null, "v": "x", "free": 2}');

select * from json_typed_variant format Native;
"#;

        #[derive(Debug, Deserialize, PartialEq)]
        struct Row<'a> {
            v: Option<serde_json::Value>,
            #[serde(borrow)]
            n: Option<&'a str>,
        }

        let data = crate::common::load("./testdata/json_typed_variant.native")?;
        let (_, block) = parse_single(&data)?;
        let reader = Json::try_from(block.mark("json")?)?;

        for row in 0..2 {
            assert_eq!(
                reader.try_read(row)?.deserialize::<serde_json::Value>()?,
                json!({"n": null, "v": null}),
                "row {row}"
            );
            assert_eq!(
                reader.try_read(row)?.deserialize::<Row<'_>>()?,
                Row { v: None, n: None }
            );
        }
        assert_eq!(
            reader.try_read(2)?.deserialize::<serde_json::Value>()?,
            json!({"n": "s", "v": 1})
        );
        assert_eq!(
            reader.try_read(3)?.deserialize::<serde_json::Value>()?,
            json!({"n": null, "v": "x", "free": 2})
        );
        Ok(())
    }

    fn check_json_fixture(
        name: &str,
        config: &DeserializeConfig,
        expected: &serde_json::Value,
    ) -> TestResult {
        let data = crate::common::load(format!("./testdata/{name}.native"))?;
        let blocks = crate::parse::block::parse_many(&data)?;
        let block = blocks
            .iter()
            .find(|block| block.num_rows > 0)
            .expect("row 0");
        let reader = Json::try_from(block.mark("j")?)?;
        let actual: serde_json::Value = reader.try_read(0)?.deserialize_with_config(config)?;
        assert_eq!(&actual, expected);
        Ok(())
    }

    #[test]
    fn typed_dynamic_null() -> TestResult {
        const _SQL: &str = r#"
select CAST('{"a":null}', 'JSON(a Dynamic)') as j format Native;
"#;

        check_json_fixture(
            "json_typed_dynamic_null",
            &DeserializeConfig::default(),
            &json!({"a": null}),
        )
    }

    #[test]
    fn array_dynamic_null() -> TestResult {
        const _SQL: &str = r#"
select CAST('{"a":[1,null,"x"]}', 'JSON(a Array(Dynamic))') as j format Native;
"#;

        check_json_fixture(
            "json_array_dynamic_null",
            &DeserializeConfig::default(),
            &json!({"a": [1, null, "x"]}),
        )
    }

    #[test]
    fn tuple_dynamic_null() -> TestResult {
        const _SQL: &str = r#"
select CAST('{"a":{"x":null}}', 'JSON(a Tuple(x Dynamic))') as j format Native;
"#;

        check_json_fixture(
            "json_tuple_dynamic_null",
            &DeserializeConfig::default(),
            &json!({"a": {"x": null}}),
        )
    }

    #[test]
    fn literal_percent_keys() -> TestResult {
        const _SQL: &str = r#"
select CAST('{"a%2Eb":2,"a%2eb":1}', 'JSON') as j format Native;
"#;

        check_json_fixture(
            "json_literal_percent_keys",
            &DeserializeConfig {
                key_mode: JsonKeyMode::Preserve,
            },
            &json!({"a%2Eb": 2, "a%2eb": 1}),
        )
    }

    #[test]
    fn escaped_keys_support_decoded_and_preserved_borrowing() -> TestResult {
        const _SQL: &str = r#"
drop table if exists json_escaped_keys;

create table json_escaped_keys
(
    j JSON(arr Array(JSON), map Map(String, JSON), tuple Tuple(doc JSON))
) engine = MergeTree order by tuple();

insert into json_escaped_keys (j) values
    ('{"a%2Eb": 1, "a%2eb": 3, "a": {"b": 2}, "arr": [{"c%2Ed": "v%2E"}], "dynamic": [{"e%2Ef": "d"}], "map": {"m%2Ek": {"c%2Ed": "m"}}, "percent%20key": 4, "tuple": {"doc": {"c%2Ed": "t"}}}');

select * from json_escaped_keys format Native;
"#;

        let data = crate::common::load("./testdata/json_escaped_keys.native")?;
        let (_, block) = parse_single(&data)?;
        let row = Json::try_from(block.mark("j")?)?.try_read(0)?;

        let decoded: BTreeMap<&str, serde_json::Value> = row.deserialize()?;
        assert_eq!(
            serde_json::to_value(&decoded)?,
            json!({
                "a.b": 1, "a": {"b": 2}, "a%2eb": 3, "percent%20key": 4,
                "arr": [{"c.d": "v%2E"}], "map": {"m%2Ek": {"c.d": "m"}},
                "tuple": {"doc": {"c.d": "t"}}, "dynamic": [{"e.f": "d"}]
            })
        );
        let preserved: BTreeMap<&str, serde_json::Value> = {
            let config = DeserializeConfig {
                key_mode: JsonKeyMode::Preserve,
            };
            row.deserialize_with_config(&config)?
        };
        assert_eq!(
            serde_json::to_value(&preserved)?,
            json!({
                "a%2Eb": 1, "a": {"b": 2}, "a%2eb": 3, "percent%20key": 4,
                "arr": [{"c%2Ed": "v%2E"}], "map": {"m%2Ek": {"c%2Ed": "m"}},
                "tuple": {"doc": {"c%2Ed": "t"}}, "dynamic": [{"e%2Ef": "d"}]
            })
        );
        assert_eq!(
            row.deserialize::<BTreeMap<&str, serde_json::Value>>()?,
            decoded
        );
        Ok(())
    }
}
