use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash};
use std::io::{self, Write};
use std::net::{Ipv4Addr, Ipv6Addr};

use rustc_hash::FxBuildHasher;

use super::{
    Bytes, ColumnWriter, ConstSizeType, ConstSizeWriter, Str, Writable, WriteColumn, WriteValue,
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
    type Key: Hash + Eq + Clone + Borrow<Self::Lookup>;
    type Lookup: Hash + Eq + ?Sized;
    fn default_key() -> Self::Key;
    fn lookup(self) -> impl Borrow<Self::Lookup>;
    fn key(self) -> Self::Key;
}

/// Implemented for the column writers ClickHouse allows under `LowCardinality`.
pub trait LcKeys: WriteValue {
    type Key: Hash + Eq + Clone;
    fn default_key() -> Self::Key;
}

impl<T: ConstSizeType> LcKeys for ConstSizeWriter<T>
where
    T::Value<'static>: LcKey,
{
    type Key = <T::Value<'static> as LcKey>::Key;
    fn default_key() -> Self::Key {
        <T::Value<'static> as LcKey>::default_key()
    }
}

impl LcKeys for Str {
    type Key = Box<str>;
    fn default_key() -> Box<str> {
        <&str>::default_key()
    }
}

impl LcKeys for Bytes {
    type Key = Box<[u8]>;
    fn default_key() -> Box<[u8]> {
        <&[u8]>::default_key()
    }
}

macro_rules! lc_key {
    ($($t:ty = $default:expr),+ $(,)?) => {$(
        impl LcKey for $t {
            type Key = Self;
            type Lookup = Self;
            fn default_key() -> Self {
                $default
            }
            fn lookup(self) -> impl Borrow<Self> {
                self
            }
            fn key(self) -> Self {
                self
            }
        }
    )+};
    ($($t:ty => $key:ty = $default:expr, |$v:ident| $expr:expr;)+) => {$(
        impl LcKey for $t {
            type Key = $key;
            type Lookup = $key;
            fn default_key() -> $key {
                $default
            }
            fn lookup(self) -> impl Borrow<$key> {
                self.key()
            }
            fn key(self) -> $key {
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
    type Key = Box<str>;
    type Lookup = str;
    fn default_key() -> Box<str> {
        "".into()
    }
    fn lookup(self) -> impl Borrow<str> {
        self
    }
    fn key(self) -> Box<str> {
        self.into()
    }
}

impl LcKey for &bstr::BStr {
    type Key = Box<[u8]>;
    type Lookup = [u8];
    fn default_key() -> Box<[u8]> {
        [].into()
    }
    fn lookup(self) -> impl Borrow<[u8]> {
        <&[u8]>::from(self)
    }
    fn key(self) -> Box<[u8]> {
        <&[u8]>::from(self).into()
    }
}

impl LcKey for &[u8] {
    type Key = Box<[u8]>;
    type Lookup = [u8];
    fn default_key() -> Box<[u8]> {
        [].into()
    }
    fn lookup(self) -> impl Borrow<[u8]> {
        self
    }
    fn key(self) -> Box<[u8]> {
        self.into()
    }
}

struct Dictionary<Keys: LcKeys, S> {
    slots: HashMap<Keys::Key, u32, S>,
    /// Slot zero of a non-nullable dictionary; kept for `clear`.
    default: Keys::Key,
    nullable: bool,
    keys: Keys,
    indices: Vec<u32>,
}

impl<Keys: LcKeys, S: BuildHasher + Default> Dictionary<Keys, S> {
    fn new(keys: Keys, nullable: bool, capacity: usize) -> Self {
        let mut this = Self {
            slots: HashMap::with_capacity_and_hasher(capacity, S::default()),
            default: Keys::default_key(),
            nullable,
            keys,
            indices: Vec::new(),
        };
        this.seed();
        this
    }

    /// Slot zero holds the default (or NULL) in every dictionary.
    fn seed(&mut self) {
        self.keys.write_default();
        if !self.nullable {
            self.slots.insert(self.default.clone(), 0);
        }
    }

    fn write<V>(
        &mut self,
        value: V,
        write_key: impl FnOnce(&mut Keys, V) -> Result<()>,
    ) -> Result<()>
    where
        V: LcKey<Key = Keys::Key>,
        Keys::Key: Borrow<V::Lookup>,
    {
        if let Some(&index) = self.slots.get(value.lookup().borrow()) {
            self.indices.push(index);
            return Ok(());
        }
        let index = u32::try_from(self.slots.len() + usize::from(self.nullable))?;
        self.slots.insert(value.key(), index);
        self.indices.push(index);
        write_key(&mut self.keys, value)
    }

    fn write_prefix(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(&LOW_CARDINALITY_VERSION.to_le_bytes())
    }

    fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
        if self.indices.is_empty() {
            return Ok(());
        }
        let dictionary_size = self.slots.len() + usize::from(self.nullable);
        let index_type = if dictionary_size <= 1 << 8 {
            TUINT8
        } else if dictionary_size <= 1 << 16 {
            TUINT16
        } else {
            TUINT32
        };
        let flags = HAS_ADDITIONAL_KEYS_BIT | NEED_UPDATE_DICTIONARY_BIT | index_type;
        out.write_all(&flags.to_le_bytes())?;
        out.write_all(&(dictionary_size as u64).to_le_bytes())?;
        self.keys.write_data(out)?;
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

    fn write_default(&mut self) {
        self.indices.push(0);
    }

    fn reserve(&mut self, rows: usize) {
        self.indices.reserve(rows);
    }

    fn clear(&mut self) {
        self.slots.clear();
        self.keys.clear();
        self.indices.clear();
        self.seed();
    }
}

/// `Lc` and `LcNullable` differ only in whether slot zero is NULL and in how they spell their
/// type name; everything else goes through `Dictionary`.
macro_rules! lc_writer {
    ($name:ident, nullable = $nullable:literal, $open:literal, $close:literal) => {
        pub struct $name<Keys: LcKeys, S = FxBuildHasher> {
            dict: Dictionary<Keys, S>,
        }

        impl<Keys: LcKeys, S: BuildHasher + Default> $name<Keys, S> {
            pub fn new(keys: Keys) -> Self {
                Self::with_capacity(keys, 0)
            }

            pub fn with_capacity(keys: Keys, capacity: usize) -> Self {
                Self {
                    dict: Dictionary::new(keys, $nullable, capacity),
                }
            }
        }

        impl<Keys: LcKeys + Default, S: BuildHasher + Default> Default for $name<Keys, S> {
            fn default() -> Self {
                Self::new(Keys::default())
            }
        }

        impl<Keys: LcKeys, S: BuildHasher + Default> ColumnWriter for $name<Keys, S> {
            fn write_type_name(&self, out: &mut String) {
                out.push_str($open);
                self.dict.keys.write_type_name(out);
                out.push_str($close);
            }

            fn write_prefix(&self, out: &mut dyn Write) -> io::Result<()> {
                self.dict.write_prefix(out)
            }

            fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
                self.dict.write_data(out)
            }

            fn write_default(&mut self) {
                self.dict.write_default();
            }

            fn flush(&mut self) -> Result<()> {
                self.dict.keys.flush()
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

impl<'a, Inner> Writable<'a> for reader::Lc<'a, Inner>
where
    Inner: Writable<'a> + 'a,
    Inner::Item: LcKey,
    Inner::Writer: LcKeys<Key = <Inner::Item as LcKey>::Key> + Default,
{
    type Writer = Lc<Inner::Writer>;
}

impl<'a, Inner, S> WriteColumn<'a, reader::Lc<'a, Inner>> for Lc<Inner::Writer, S>
where
    Inner: Writable<'a> + 'a,
    Inner::Item: LcKey,
    Inner::Writer: LcKeys<Key = <Inner::Item as LcKey>::Key> + Default,
    S: BuildHasher + Default,
{
    fn for_reader(reader: &reader::Lc<'a, Inner>) -> Self {
        match &reader.dict {
            Some(dict) => Self::with_capacity(Inner::Writer::for_reader(dict), dict.len()),
            // An empty source column has no dictionary reader to copy parameters from.
            None => Self::default(),
        }
    }

    fn write_item(&mut self, item: Inner::Item) -> Result<()> {
        self.dict
            .write(item, <Inner::Writer as WriteColumn<Inner>>::write_item)
    }
}

impl<Keys, S> WriteValue for Lc<Keys, S>
where
    Keys: LcKeys,
    for<'a> Keys::Value<'a>: LcKey<Key = Keys::Key>,
    for<'a> Keys::Key: Borrow<<Keys::Value<'a> as LcKey>::Lookup>,
    S: BuildHasher + Default,
{
    type Value<'a> = Keys::Value<'a>;

    fn write(&mut self, value: Keys::Value<'_>) -> Result<()> {
        self.dict.write(value, Keys::write)
    }
}

impl<'a, Inner> Writable<'a> for reader::LcNullable<'a, Inner>
where
    Inner: Writable<'a> + 'a,
    Inner::Item: LcKey,
    Inner::Writer: LcKeys<Key = <Inner::Item as LcKey>::Key> + Default,
{
    type Writer = LcNullable<Inner::Writer>;
}

impl<'a, Inner, S> WriteColumn<'a, reader::LcNullable<'a, Inner>> for LcNullable<Inner::Writer, S>
where
    Inner: Writable<'a> + 'a,
    Inner::Item: LcKey,
    Inner::Writer: LcKeys<Key = <Inner::Item as LcKey>::Key> + Default,
    S: BuildHasher + Default,
{
    fn for_reader(reader: &reader::LcNullable<'a, Inner>) -> Self {
        match &reader.dict {
            Some(dict) => Self::with_capacity(Inner::Writer::for_reader(dict), dict.len()),
            None => Self::default(),
        }
    }

    fn write_item(&mut self, item: Option<Inner::Item>) -> Result<()> {
        match item {
            Some(item) => self
                .dict
                .write(item, <Inner::Writer as WriteColumn<Inner>>::write_item),
            None => {
                self.dict.write_default();
                Ok(())
            }
        }
    }
}

impl<Keys, S> WriteValue for LcNullable<Keys, S>
where
    Keys: LcKeys,
    for<'a> Keys::Value<'a>: LcKey<Key = Keys::Key>,
    for<'a> Keys::Key: Borrow<<Keys::Value<'a> as LcKey>::Lookup>,
    S: BuildHasher + Default,
{
    type Value<'a> = Option<Keys::Value<'a>>;

    fn write(&mut self, value: Option<Keys::Value<'_>>) -> Result<()> {
        match value {
            Some(value) => self.dict.write(value, Keys::write),
            None => {
                self.dict.write_default();
                Ok(())
            }
        }
    }
}
