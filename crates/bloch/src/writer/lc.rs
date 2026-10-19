use std::borrow::{Borrow, Cow};
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash};
use std::io::{self, Write};
use std::net::{Ipv4Addr, Ipv6Addr};

use rustc_hash::FxBuildHasher;

use super::{
    Bytes, ColumnWriter, DefaultWriter, FixedWidth, FixedWidthColumn, FromReader, Str, ValueWriter,
    write_le,
};
use crate::{
    Result,
    parse::consts::{
        HAS_ADDITIONAL_KEYS_BIT, LOW_CARDINALITY_VERSION, NEED_UPDATE_DICTIONARY_BIT, TUINT8,
        TUINT16, TUINT32,
    },
    reader,
};

pub trait LcKey: Copy {
    type Owned: Hash + Eq + Clone + Borrow<Self::Borrowed>;
    type Borrowed: Hash + Eq + ?Sized;
    /// Key of ClickHouse's default for the type: slot zero of a non-nullable dictionary.
    fn default_key() -> Self::Owned;
    fn as_borrowed(self) -> impl Borrow<Self::Borrowed>;
    fn into_owned(self) -> Self::Owned;
}

pub trait LcColumn: ValueWriter {
    type Key: Hash + Eq + Clone;
    fn default_key() -> Self::Key;
}

impl<T: FixedWidth> LcColumn for FixedWidthColumn<T>
where
    T::Value<'static>: LcKey,
{
    type Key = <T::Value<'static> as LcKey>::Owned;
    fn default_key() -> Self::Key {
        <T::Value<'static> as LcKey>::default_key()
    }
}

impl LcColumn for Str {
    type Key = Box<str>;
    fn default_key() -> Box<str> {
        <&str>::default_key()
    }
}

impl LcColumn for Bytes {
    type Key = Box<[u8]>;
    fn default_key() -> Box<[u8]> {
        <&[u8]>::default_key()
    }
}

macro_rules! lc_key {
    ($($t:ty = $default:expr),+ $(,)?) => {$(
        impl LcKey for $t {
            type Owned = Self;
            type Borrowed = Self;
            fn default_key() -> Self {
                $default
            }
            fn as_borrowed(self) -> impl Borrow<Self> {
                self
            }
            fn into_owned(self) -> Self {
                self
            }
        }
    )+};
    ($($t:ty => $key:ty = $default:expr, |$v:ident| $expr:expr;)+) => {$(
        impl LcKey for $t {
            type Owned = $key;
            type Borrowed = $key;
            fn default_key() -> $key {
                $default
            }
            fn as_borrowed(self) -> impl Borrow<$key> {
                self.into_owned()
            }
            fn into_owned(self) -> $key {
                let $v = self;
                $expr
            }
        }
    )+};
}

lc_key!(
    i8 = 0,
    i16 = 0,
    i32 = 0,
    i64 = 0,
    i128 = 0,
    u8 = 0,
    u16 = 0,
    u32 = 0,
    u64 = 0,
    u128 = 0,
    usize = 0,
    bool = false,
    uuid::Uuid = uuid::Uuid::nil(),
    Ipv4Addr = Ipv4Addr::UNSPECIFIED,
    Ipv6Addr = Ipv6Addr::UNSPECIFIED,
    chrono::NaiveDate = chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
    chrono::TimeDelta = chrono::TimeDelta::zero(),
    rust_decimal::Decimal = rust_decimal::Decimal::ZERO,
);
lc_key!(
    f32 => u32 = 0, |v| v.to_bits();
    f64 => u64 = 0, |v| v.to_bits();
    half::bf16 => u16 = 0, |v| v.to_bits();
    chrono::DateTime<chrono_tz::Tz> => (i64, u32) = (0, 0), |v| (v.timestamp(), v.timestamp_subsec_nanos());
    chrono::DateTime<chrono::Utc> => (i64, u32) = (0, 0), |v| (v.timestamp(), v.timestamp_subsec_nanos());
);

impl LcKey for &str {
    type Owned = Box<str>;
    type Borrowed = str;
    fn default_key() -> Box<str> {
        "".into()
    }
    fn as_borrowed(self) -> impl Borrow<str> {
        self
    }
    fn into_owned(self) -> Box<str> {
        self.into()
    }
}

impl LcKey for &bstr::BStr {
    type Owned = Box<[u8]>;
    type Borrowed = [u8];
    fn default_key() -> Box<[u8]> {
        [].into()
    }
    fn as_borrowed(self) -> impl Borrow<[u8]> {
        <&[u8]>::from(self)
    }
    fn into_owned(self) -> Box<[u8]> {
        <&[u8]>::from(self).into()
    }
}

impl LcKey for &[u8] {
    type Owned = Box<[u8]>;
    type Borrowed = [u8];
    fn default_key() -> Box<[u8]> {
        [].into()
    }
    fn as_borrowed(self) -> impl Borrow<[u8]> {
        self
    }
    fn into_owned(self) -> Box<[u8]> {
        self.into()
    }
}

struct Dictionary<Inner: LcColumn, S> {
    slots: HashMap<Inner::Key, u32, S>,
    /// Slot zero of a non-nullable dictionary; kept for `clear`.
    default: Inner::Key,
    nullable: bool,
    inner: Inner,
    indices: Vec<u32>,
}

impl<Inner: LcColumn, S: BuildHasher + Default> Dictionary<Inner, S> {
    fn new(inner: Inner, nullable: bool, capacity: usize) -> Self {
        let mut this = Self {
            slots: HashMap::with_capacity_and_hasher(capacity, S::default()),
            default: Inner::default_key(),
            nullable,
            inner,
            indices: Vec::new(),
        };
        this.seed();
        this
    }

    /// Slot zero holds the default (or NULL) in every dictionary.
    fn seed(&mut self) {
        self.inner.push_default();
        if !self.nullable {
            self.slots.insert(self.default.clone(), 0);
        }
    }

    /// `write` appends `value` to the inner column; it runs only on a dictionary miss.
    fn write<V>(&mut self, value: V, write: impl FnOnce(&mut Inner, V) -> Result<()>) -> Result<()>
    where
        V: LcKey<Owned = Inner::Key>,
        Inner::Key: Borrow<V::Borrowed>,
    {
        if let Some(&index) = self.slots.get(value.as_borrowed().borrow()) {
            self.indices.push(index);
            return Ok(());
        }
        let index = u32::try_from(self.slots.len() + usize::from(self.nullable))?;
        self.slots.insert(value.into_owned(), index);
        self.indices.push(index);
        write(&mut self.inner, value)
    }

    fn write_slice<V>(
        &mut self,
        values: &[V],
        write: impl Fn(&mut Inner, V) -> Result<()>,
    ) -> Result<()>
    where
        V: LcKey<Owned = Inner::Key>,
        Inner::Key: Borrow<V::Borrowed>,
    {
        self.indices.reserve(values.len());
        for &value in values {
            if let Some(&index) = self.slots.get(value.as_borrowed().borrow()) {
                self.indices.push(index);
            } else {
                let index = u32::try_from(self.slots.len() + usize::from(self.nullable))?;
                self.slots.insert(value.into_owned(), index);
                self.indices.push(index);
                write(&mut self.inner, value)?;
            }
        }
        Ok(())
    }

    fn write_prefix(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(&LOW_CARDINALITY_VERSION.to_le_bytes())
    }

    const fn index_type(dictionary_size: usize) -> u64 {
        if dictionary_size <= 1 << 8 {
            TUINT8
        } else if dictionary_size <= 1 << 16 {
            TUINT16
        } else {
            TUINT32
        }
    }

    fn dictionary_size(&self) -> usize {
        self.slots.len() + usize::from(self.nullable)
    }

    fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
        if self.indices.is_empty() {
            return Ok(());
        }
        let dictionary_size = self.dictionary_size();
        let index_type = Self::index_type(dictionary_size);
        let flags = HAS_ADDITIONAL_KEYS_BIT | NEED_UPDATE_DICTIONARY_BIT | index_type;
        out.write_all(&flags.to_le_bytes())?;
        out.write_all(&(dictionary_size as u64).to_le_bytes())?;
        self.inner.write_data(out)?;
        out.write_all(&(self.indices.len() as u64).to_le_bytes())?;
        match index_type {
            TUINT8 => write_le(&self.indices, |index| [index.to_le_bytes()[0]], out),
            TUINT16 => write_le(
                &self.indices,
                |index| {
                    let [lo, hi, ..] = index.to_le_bytes();
                    [lo, hi]
                },
                out,
            ),
            _ => write_le(&self.indices, u32::to_le_bytes, out),
        }
    }

    fn data_len(&self) -> usize {
        if self.indices.is_empty() {
            return 0;
        }
        let index_width = match Self::index_type(self.dictionary_size()) {
            TUINT8 => 1,
            TUINT16 => 2,
            _ => 4,
        };
        // flags + dictionary size + keys + index count + indices
        3 * size_of::<u64>() + self.inner.data_len() + self.indices.len() * index_width
    }

    fn push_default(&mut self) {
        self.indices.push(0);
    }

    fn reserve(&mut self, rows: usize) {
        self.indices.reserve(rows);
    }

    fn clear(&mut self) {
        self.slots.clear();
        self.inner.clear();
        self.indices.clear();
        self.seed();
    }
}

/// `Lc` and `LcNullable` differ only in whether slot zero is NULL and in how they spell their
/// type name; everything else goes through `Dictionary`.
macro_rules! lc_writer {
    ($name:ident, nullable = $nullable:literal, $open:literal, $close:literal) => {
        pub struct $name<Inner: LcColumn, S = FxBuildHasher> {
            dict: Dictionary<Inner, S>,
        }

        impl<Inner: LcColumn, S: BuildHasher + Default> $name<Inner, S> {
            pub fn new(inner: Inner) -> Self {
                Self::with_capacity(inner, 0)
            }

            pub fn with_capacity(inner: Inner, capacity: usize) -> Self {
                Self {
                    dict: Dictionary::new(inner, $nullable, capacity),
                }
            }
        }

        impl<Inner: LcColumn + Default, S: BuildHasher + Default> Default for $name<Inner, S> {
            fn default() -> Self {
                Self::new(Inner::default())
            }
        }

        impl<Inner: LcColumn, S: BuildHasher + Default> ColumnWriter for $name<Inner, S> {
            fn type_name(&self) -> Cow<'static, str> {
                format!(concat!($open, "{}", $close), self.dict.inner.type_name()).into()
            }

            fn write_prefix(&self, out: &mut dyn Write) -> io::Result<()> {
                self.dict.write_prefix(out)
            }

            fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
                self.dict.write_data(out)
            }

            fn data_len(&self) -> usize {
                self.dict.data_len()
            }

            fn push_default(&mut self) {
                self.dict.push_default();
            }

            fn flush(&mut self) -> Result<()> {
                self.dict.inner.flush()
            }

            fn reserve(&mut self, rows: usize) {
                self.dict.reserve(rows);
            }

            fn clear(&mut self) {
                self.dict.clear();
            }
        }
    };
}

lc_writer!(Lc, nullable = false, "LowCardinality(", ")");
lc_writer!(
    LcNullable,
    nullable = true,
    "LowCardinality(Nullable(",
    "))"
);

impl<'a, Inner> DefaultWriter<'a> for reader::Lc<'a, Inner>
where
    Inner: DefaultWriter<'a> + 'a,
    Inner::Item: LcKey,
    Inner::Writer: LcColumn<Key = <Inner::Item as LcKey>::Owned> + Default,
{
    type Writer = Lc<Inner::Writer>;
}

impl<'a, Inner, S> FromReader<'a, reader::Lc<'a, Inner>> for Lc<Inner::Writer, S>
where
    Inner: DefaultWriter<'a> + 'a,
    Inner::Item: LcKey,
    Inner::Writer: LcColumn<Key = <Inner::Item as LcKey>::Owned> + Default,
    S: BuildHasher + Default,
{
    fn from_reader(reader: &reader::Lc<'a, Inner>) -> Self {
        match &reader.dict {
            Some(dict) => Self::with_capacity(Inner::Writer::from_reader(dict), dict.len()),
            // An empty source column has no dictionary reader to copy parameters from.
            None => Self::default(),
        }
    }

    fn write_item(&mut self, item: Inner::Item) -> Result<()> {
        self.dict
            .write(item, <Inner::Writer as FromReader<Inner>>::write_item)
    }
}

impl<Inner, S> ValueWriter for Lc<Inner, S>
where
    Inner: LcColumn,
    for<'a> Inner::Value<'a>: LcKey<Owned = Inner::Key>,
    for<'a> Inner::Key: Borrow<<Inner::Value<'a> as LcKey>::Borrowed>,
    S: BuildHasher + Default,
{
    type Value<'a> = Inner::Value<'a>;

    fn write(&mut self, value: Inner::Value<'_>) -> Result<()> {
        self.dict.write(value, Inner::write)
    }

    fn write_slice(&mut self, values: &[Self::Value<'_>]) -> Result<()> {
        self.dict.write_slice(values, Inner::write)
    }
}

impl<'a, Inner> DefaultWriter<'a> for reader::LcNullable<'a, Inner>
where
    Inner: DefaultWriter<'a> + 'a,
    Inner::Item: LcKey,
    Inner::Writer: LcColumn<Key = <Inner::Item as LcKey>::Owned> + Default,
{
    type Writer = LcNullable<Inner::Writer>;
}

impl<'a, Inner, S> FromReader<'a, reader::LcNullable<'a, Inner>> for LcNullable<Inner::Writer, S>
where
    Inner: DefaultWriter<'a> + 'a,
    Inner::Item: LcKey,
    Inner::Writer: LcColumn<Key = <Inner::Item as LcKey>::Owned> + Default,
    S: BuildHasher + Default,
{
    fn from_reader(reader: &reader::LcNullable<'a, Inner>) -> Self {
        match &reader.dict {
            Some(dict) => Self::with_capacity(Inner::Writer::from_reader(dict), dict.len()),
            None => Self::default(),
        }
    }

    fn write_item(&mut self, item: Option<Inner::Item>) -> Result<()> {
        match item {
            Some(item) => self
                .dict
                .write(item, <Inner::Writer as FromReader<Inner>>::write_item),
            None => {
                self.dict.push_default();
                Ok(())
            }
        }
    }
}

impl<Inner, S> ValueWriter for LcNullable<Inner, S>
where
    Inner: LcColumn,
    for<'a> Inner::Value<'a>: LcKey<Owned = Inner::Key>,
    for<'a> Inner::Key: Borrow<<Inner::Value<'a> as LcKey>::Borrowed>,
    S: BuildHasher + Default,
{
    type Value<'a> = Option<Inner::Value<'a>>;

    fn write(&mut self, value: Option<Inner::Value<'_>>) -> Result<()> {
        match value {
            Some(value) => self.dict.write(value, Inner::write),
            None => {
                self.dict.push_default();
                Ok(())
            }
        }
    }
}
