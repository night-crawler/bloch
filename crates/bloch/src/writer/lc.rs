use std::borrow::Borrow;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hash};
use std::io::{self, Write};
use std::net::{Ipv4Addr, Ipv6Addr};

use rustc_hash::FxBuildHasher;

use super::{ColumnWriter, Writable, WriteColumn, write_le};
use crate::{
    Result,
    parse::consts::{
        HAS_ADDITIONAL_KEYS_BIT, LOW_CARDINALITY_VERSION, NEED_UPDATE_DICTIONARY_BIT, TUINT8,
        TUINT16, TUINT32,
    },
    reader,
};

pub trait LcDictionaryKeyDefault: Hash + Eq {
    fn default_key() -> Self;
}

pub trait LcKey: Copy {
    type Key: LcDictionaryKeyDefault + Borrow<Self::Lookup>;
    type Lookup: Hash + Eq + ?Sized;
    type Value: Borrow<Self::Lookup> + Into<Self::Key> + Copy;
    fn lc_key(self) -> Self::Value;
}

macro_rules! lc_key {
    ($($t:ty = $default:expr),+ $(,)?) => {$(
        impl LcDictionaryKeyDefault for $t {
            fn default_key() -> Self {
                $default
            }
        }
        impl LcKey for $t {
            type Key = Self;
            type Lookup = Self;
            type Value = Self;
            fn lc_key(self) -> Self {
                self
            }
        }
    )+};
    ($($t:ty => $key:ty, |$v:ident| $expr:expr;)+) => {$(
        impl LcKey for $t {
            type Key = $key;
            type Lookup = $key;
            type Value = $key;
            fn lc_key(self) -> $key {
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
    f32 => u32, |v| v.to_bits();
    f64 => u64, |v| v.to_bits();
    half::bf16 => u16, |v| v.to_bits();
    chrono::DateTime<chrono_tz::Tz> => (i64, u32), |v| (v.timestamp(), v.timestamp_subsec_nanos());
);

impl LcDictionaryKeyDefault for (i64, u32) {
    fn default_key() -> Self {
        (0, 0)
    }
}

impl LcDictionaryKeyDefault for Box<str> {
    fn default_key() -> Self {
        "".into()
    }
}

impl LcDictionaryKeyDefault for Box<[u8]> {
    fn default_key() -> Self {
        [].into()
    }
}

impl LcKey for &str {
    type Key = Box<str>;
    type Lookup = str;
    type Value = Self;
    fn lc_key(self) -> Self {
        self
    }
}

impl<'a> LcKey for &'a bstr::BStr {
    type Key = Box<[u8]>;
    type Lookup = [u8];
    type Value = &'a [u8];
    fn lc_key(self) -> &'a [u8] {
        self
    }
}

struct Dictionary<Key: LcDictionaryKeyDefault, Keys: ColumnWriter, S: BuildHasher + Default> {
    slots: HashMap<Key, u32, S>,
    nullable: bool,
    keys: Keys,
    indices: Vec<u32>,
}

impl<Key: LcDictionaryKeyDefault, Keys: ColumnWriter, S: BuildHasher + Default>
    Dictionary<Key, Keys, S>
{
    fn new(keys: Keys, nullable: bool, capacity: usize) -> Self {
        let mut this = Self {
            slots: HashMap::with_capacity_and_hasher(capacity, S::default()),
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
            self.slots.insert(Key::default_key(), 0);
        }
    }

    fn insert<Q>(&mut self, key: &Q, own_key: impl FnOnce() -> Key) -> Result<bool>
    where
        Key: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let (index, inserted) = match self.slots.get(key) {
            Some(&index) => (index, false),
            None => {
                let index = u32::try_from(self.slots.len() + usize::from(self.nullable))?;
                self.slots.insert(own_key(), index);
                (index, true)
            }
        };
        self.indices.push(index);
        Ok(inserted)
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

pub struct Lc<
    K: LcDictionaryKeyDefault,
    Keys: ColumnWriter,
    S: BuildHasher + Default = FxBuildHasher,
> {
    dict: Dictionary<K, Keys, S>,
}

impl<K: LcDictionaryKeyDefault, Keys: ColumnWriter, S: BuildHasher + Default> Lc<K, Keys, S> {
    pub fn new(keys: Keys) -> Self {
        Self::with_capacity(keys, 0)
    }

    pub fn with_capacity(keys: Keys, capacity: usize) -> Self {
        Self {
            dict: Dictionary::new(keys, false, capacity),
        }
    }
}

impl<K: LcDictionaryKeyDefault, Keys: ColumnWriter + Default, S: BuildHasher + Default> Default
    for Lc<K, Keys, S>
{
    fn default() -> Self {
        Self::new(Keys::default())
    }
}

impl<K: LcDictionaryKeyDefault, Keys: ColumnWriter, S: BuildHasher + Default> ColumnWriter
    for Lc<K, Keys, S>
{
    fn write_type_name(&self, out: &mut String) {
        out.push_str("LowCardinality(");
        self.dict.keys.write_type_name(out);
        out.push(')');
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

impl<'a, Inner> Writable<'a> for reader::Lc<'a, Inner>
where
    Inner: Writable<'a> + 'a,
    Inner::Item: LcKey,
    Inner::Writer: Default,
{
    type Writer = Lc<<Inner::Item as LcKey>::Key, Inner::Writer>;
}

impl<'a, Inner, S> WriteColumn<'a, reader::Lc<'a, Inner>>
    for Lc<<Inner::Item as LcKey>::Key, Inner::Writer, S>
where
    Inner: Writable<'a> + 'a,
    Inner::Item: LcKey,
    Inner::Writer: Default,
    S: BuildHasher + Default,
{
    fn for_reader(reader: &reader::Lc<'a, Inner>) -> Self {
        match &reader.dict {
            Some(dict) => Self::with_capacity(Inner::Writer::for_reader(dict), dict.len()),
            // An empty source column has no dictionary reader to copy parameters from.
            None => Self::default(),
        }
    }

    fn write(&mut self, item: Inner::Item) -> Result<()> {
        let key = item.lc_key();
        if self
            .dict
            .insert::<<Inner::Item as LcKey>::Lookup>(key.borrow(), || key.into())?
        {
            self.dict.keys.write(item)?;
        }
        Ok(())
    }
}

pub struct LcNullable<
    K: LcDictionaryKeyDefault,
    Keys: ColumnWriter,
    S: BuildHasher + Default = FxBuildHasher,
> {
    dict: Dictionary<K, Keys, S>,
}

impl<K: LcDictionaryKeyDefault, Keys: ColumnWriter, S: BuildHasher + Default>
    LcNullable<K, Keys, S>
{
    pub fn new(keys: Keys) -> Self {
        Self::with_capacity(keys, 0)
    }

    pub fn with_capacity(keys: Keys, capacity: usize) -> Self {
        Self {
            dict: Dictionary::new(keys, true, capacity),
        }
    }
}

impl<K: LcDictionaryKeyDefault, Keys: ColumnWriter + Default, S: BuildHasher + Default> Default
    for LcNullable<K, Keys, S>
{
    fn default() -> Self {
        Self::new(Keys::default())
    }
}

impl<K: LcDictionaryKeyDefault, Keys: ColumnWriter, S: BuildHasher + Default> ColumnWriter
    for LcNullable<K, Keys, S>
{
    fn write_type_name(&self, out: &mut String) {
        out.push_str("LowCardinality(Nullable(");
        self.dict.keys.write_type_name(out);
        out.push_str("))");
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

impl<'a, Inner> Writable<'a> for reader::LcNullable<'a, Inner>
where
    Inner: Writable<'a> + 'a,
    Inner::Item: LcKey,
    Inner::Writer: Default,
{
    type Writer = LcNullable<<Inner::Item as LcKey>::Key, Inner::Writer>;
}

impl<'a, Inner, S> WriteColumn<'a, reader::LcNullable<'a, Inner>>
    for LcNullable<<Inner::Item as LcKey>::Key, Inner::Writer, S>
where
    Inner: Writable<'a> + 'a,
    Inner::Item: LcKey,
    Inner::Writer: Default,
    S: BuildHasher + Default,
{
    fn for_reader(reader: &reader::LcNullable<'a, Inner>) -> Self {
        match &reader.dict {
            Some(dict) => Self::with_capacity(Inner::Writer::for_reader(dict), dict.len()),
            None => Self::default(),
        }
    }

    fn write(&mut self, item: Option<Inner::Item>) -> Result<()> {
        match item {
            Some(item) => {
                let key = item.lc_key();
                if self
                    .dict
                    .insert::<<Inner::Item as LcKey>::Lookup>(key.borrow(), || key.into())?
                {
                    self.dict.keys.write(item)?;
                }
            }
            None => self.dict.write_default(),
        }
        Ok(())
    }
}
