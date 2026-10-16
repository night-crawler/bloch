use std::{
    io::{self, Write},
    marker::PhantomData,
};

use super::{
    ColumnWriter, ElementWriters, VariantName, Writable, WriteColumn, WriteValue, WriteVariant,
    write_offsets,
};
use crate::{Result, reader};

pub struct Nullable<Inner: ColumnWriter> {
    nulls: Vec<u8>,
    inner: Inner,
}

impl<Inner: ColumnWriter> Nullable<Inner> {
    pub const fn new(inner: Inner) -> Self {
        Self {
            nulls: Vec::new(),
            inner,
        }
    }
}

impl<Inner: ColumnWriter + Default> Default for Nullable<Inner> {
    fn default() -> Self {
        Self::new(Inner::default())
    }
}

impl<Inner: ColumnWriter> ColumnWriter for Nullable<Inner> {
    fn write_type_name(&self, out: &mut String) {
        out.push_str("Nullable(");
        self.inner.write_type_name(out);
        out.push(')');
    }

    fn write_prefix(&self, out: &mut dyn Write) -> io::Result<()> {
        self.inner.write_prefix(out)
    }

    fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(&self.nulls)?;
        self.inner.write_data(out)
    }

    fn write_default(&mut self) {
        self.nulls.push(1);
        self.inner.write_default();
    }

    fn flush(&mut self) -> Result<()> {
        self.inner.flush()
    }

    fn reserve(&mut self, rows: usize) {
        self.nulls.reserve(rows);
        self.inner.reserve(rows);
    }

    fn clear(&mut self) {
        self.nulls.clear();
        self.inner.clear();
    }
}

impl<'a, Inner: Writable<'a> + 'a> Writable<'a> for reader::Nullable<'a, Inner> {
    type Writer = Nullable<Inner::Writer>;
}

impl<'a, Inner: Writable<'a> + 'a> WriteColumn<'a, reader::Nullable<'a, Inner>>
    for Nullable<Inner::Writer>
{
    fn for_reader(reader: &reader::Nullable<'a, Inner>) -> Self {
        Self::new(Inner::Writer::for_reader(&reader.inner))
    }

    fn write_item(&mut self, item: Option<Inner::Item>) -> Result<()> {
        match item {
            Some(value) => {
                self.nulls.push(0);
                self.inner.write_item(value)
            }
            None => {
                self.write_default();
                Ok(())
            }
        }
    }
}

impl<Inner: WriteValue> WriteValue for Nullable<Inner> {
    type Value<'a> = Option<Inner::Value<'a>>;

    fn write(&mut self, value: Option<Inner::Value<'_>>) -> Result<()> {
        match value {
            Some(value) => {
                self.nulls.push(0);
                self.inner.write(value)
            }
            None => {
                self.write_default();
                Ok(())
            }
        }
    }
}

pub struct Array<Values: ColumnWriter> {
    offsets: Vec<u64>,
    values: Values,
}

impl<Values: ColumnWriter> Array<Values> {
    pub const fn new(values: Values) -> Self {
        Self {
            offsets: Vec::new(),
            values,
        }
    }

    pub fn write_iter<'a>(
        &mut self,
        values: impl IntoIterator<Item = Values::Value<'a>>,
    ) -> Result<()>
    where
        Values: WriteValue,
    {
        self.write_with(values, Values::write)
    }

    fn write_with<T>(
        &mut self,
        values: impl IntoIterator<Item = T>,
        mut write_value: impl FnMut(&mut Values, T) -> Result<()>,
    ) -> Result<()> {
        let mut end = self.offsets.last().copied().unwrap_or(0);
        for value in values {
            write_value(&mut self.values, value)?;
            end += 1;
        }
        self.offsets.push(end);
        Ok(())
    }
}

impl<Values: ColumnWriter + Default> Default for Array<Values> {
    fn default() -> Self {
        Self::new(Values::default())
    }
}

impl<Values: ColumnWriter> ColumnWriter for Array<Values> {
    fn write_type_name(&self, out: &mut String) {
        out.push_str("Array(");
        self.values.write_type_name(out);
        out.push(')');
    }

    fn write_prefix(&self, out: &mut dyn Write) -> io::Result<()> {
        self.values.write_prefix(out)
    }

    fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
        write_offsets(&self.offsets, out)?;
        self.values.write_data(out)
    }

    fn write_default(&mut self) {
        self.offsets.push(self.offsets.last().copied().unwrap_or(0));
    }

    fn flush(&mut self) -> Result<()> {
        self.values.flush()
    }

    /// We reserve only for offsets: how many elements the rows will hold is unknown.
    fn reserve(&mut self, rows: usize) {
        self.offsets.reserve(rows);
    }

    fn clear(&mut self) {
        self.offsets.clear();
        self.values.clear();
    }
}

impl<'a, Inner: Writable<'a> + 'a> Writable<'a> for reader::Array<'a, Inner> {
    type Writer = Array<Inner::Writer>;
}

impl<'a, Inner: Writable<'a> + 'a> WriteColumn<'a, reader::Array<'a, Inner>>
    for Array<Inner::Writer>
{
    fn for_reader(reader: &reader::Array<'a, Inner>) -> Self {
        let mut values = Inner::Writer::for_reader(&reader.values);
        values.reserve(reader.values.len());
        Self::new(values)
    }

    fn write_item(&mut self, item: reader::ArrayIter<'a, Inner>) -> Result<()> {
        self.write_with(item, |values, item| values.write_item(item?))
    }
}

impl<Values> WriteValue for Array<Values>
where
    Values: WriteValue,
    for<'a> Values::Value<'a>: Copy,
{
    type Value<'a> = &'a [Values::Value<'a>];

    fn write(&mut self, values: Self::Value<'_>) -> Result<()> {
        self.write_iter(values.iter().copied())
    }
}

pub struct Map<Keys: ColumnWriter, Values: ColumnWriter> {
    offsets: Vec<u64>,
    keys: Keys,
    values: Values,
}

impl<Keys: ColumnWriter, Values: ColumnWriter> Map<Keys, Values> {
    pub const fn new(keys: Keys, values: Values) -> Self {
        Self {
            offsets: Vec::new(),
            keys,
            values,
        }
    }
}

impl<Keys: ColumnWriter + Default, Values: ColumnWriter + Default> Default for Map<Keys, Values> {
    fn default() -> Self {
        Self::new(Keys::default(), Values::default())
    }
}

impl<Keys: ColumnWriter, Values: ColumnWriter> ColumnWriter for Map<Keys, Values> {
    fn write_type_name(&self, out: &mut String) {
        out.push_str("Map(");
        self.keys.write_type_name(out);
        out.push(',');
        self.values.write_type_name(out);
        out.push(')');
    }

    fn write_prefix(&self, out: &mut dyn Write) -> io::Result<()> {
        self.keys.write_prefix(out)?;
        self.values.write_prefix(out)
    }

    fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
        write_offsets(&self.offsets, out)?;
        self.keys.write_data(out)?;
        self.values.write_data(out)
    }

    fn write_default(&mut self) {
        self.offsets.push(self.offsets.last().copied().unwrap_or(0));
    }

    fn flush(&mut self) -> Result<()> {
        self.keys.flush()?;
        self.values.flush()
    }

    fn reserve(&mut self, rows: usize) {
        self.offsets.reserve(rows);
    }

    fn clear(&mut self) {
        self.offsets.clear();
        self.keys.clear();
        self.values.clear();
    }
}

impl<'a, K, V> Writable<'a> for reader::Map<'a, K, V>
where
    K: Writable<'a> + 'a,
    V: Writable<'a> + 'a,
{
    type Writer = Map<K::Writer, V::Writer>;
}

impl<'a, K, V> WriteColumn<'a, reader::Map<'a, K, V>> for Map<K::Writer, V::Writer>
where
    K: Writable<'a> + 'a,
    V: Writable<'a> + 'a,
{
    fn for_reader(reader: &reader::Map<'a, K, V>) -> Self {
        let mut keys = K::Writer::for_reader(&reader.keys);
        keys.reserve(reader.keys.len());
        let mut values = V::Writer::for_reader(&reader.values);
        values.reserve(reader.values.len());
        Self::new(keys, values)
    }

    fn write_item(&mut self, item: reader::MapIter<'a, K, V>) -> Result<()> {
        let mut end = self.offsets.last().copied().unwrap_or(0);
        for entry in item {
            let (key, value) = entry?;
            self.keys.write_item(key)?;
            self.values.write_item(value)?;
            end += 1;
        }
        self.offsets.push(end);
        Ok(())
    }
}

impl<Keys, Values> WriteValue for Map<Keys, Values>
where
    Keys: WriteValue,
    Values: WriteValue,
    for<'a> Keys::Value<'a>: Copy,
    for<'a> Values::Value<'a>: Copy,
{
    type Value<'a> = &'a [(Keys::Value<'a>, Values::Value<'a>)];

    fn write(&mut self, entries: Self::Value<'_>) -> Result<()> {
        for &(key, value) in entries {
            self.keys.write(key)?;
            self.values.write(value)?;
        }

        let end = self.offsets.last().copied().unwrap_or(0) + entries.len() as u64;

        self.offsets.push(end);
        Ok(())
    }
}

macro_rules! tuples {
    ($($idx:tt => $t:ident),+) => {
        impl<$($t: ColumnWriter,)+> ElementWriters for ($($t,)+) {
            fn write_type_names(&self, out: &mut Vec<String>) {
                $(
                    let mut name = String::new();
                    self.$idx.write_type_name(&mut name);
                    out.push(name);
                )+
            }

            fn write_prefix(&self, out: &mut dyn Write) -> io::Result<()> {
                $(self.$idx.write_prefix(out)?;)+
                Ok(())
            }

            fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
                $(self.$idx.write_data(out)?;)+
                Ok(())
            }

            fn write_default(&mut self) {
                $(self.$idx.write_default();)+
            }

            fn reserve(&mut self, rows: usize) {
                $(self.$idx.reserve(rows);)+
            }

            fn clear(&mut self) {
                $(self.$idx.clear();)+
            }

            fn flush(&mut self) -> Result<()> {
                $(self.$idx.flush()?;)+
                Ok(())
            }
        }

        impl<$($t: ColumnWriter,)+> ColumnWriter for ($($t,)+) {
            fn write_type_name(&self, out: &mut String) {
                let mut names = Vec::new();
                self.write_type_names(&mut names);
                out.push_str("Tuple(");
                out.push_str(&names.join(","));
                out.push(')');
            }

            fn write_prefix(&self, out: &mut dyn Write) -> io::Result<()> {
                ElementWriters::write_prefix(self, out)
            }

            fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
                ElementWriters::write_data(self, out)
            }

            fn write_default(&mut self) {
                ElementWriters::write_default(self);
            }

            fn reserve(&mut self, rows: usize) {
                ElementWriters::reserve(self, rows);
            }

            fn clear(&mut self) {
                ElementWriters::clear(self);
            }

            fn flush(&mut self) -> Result<()> {
                ElementWriters::flush(self)
            }
        }

        impl<'a, $($t: Writable<'a> + 'a,)+> Writable<'a> for reader::Tuple<($($t,)+)> {
            type Writer = ($($t::Writer,)+);
        }

        impl<'a, $($t: Writable<'a> + 'a,)+> WriteColumn<'a, reader::Tuple<($($t,)+)>>
            for ($($t::Writer,)+)
        {
            fn for_reader(reader: &reader::Tuple<($($t,)+)>) -> Self {
                ($($t::Writer::for_reader(&reader.0.$idx),)+)
            }

            fn write_item(&mut self, item: ($($t::Item,)+)) -> Result<()> {
                $(self.$idx.write_item(item.$idx)?;)+
                Ok(())
            }
        }

        impl<$($t: WriteValue,)+> WriteValue for ($($t,)+) {
            type Value<'a> = ($($t::Value<'a>,)+);

            fn write(&mut self, value: Self::Value<'_>) -> Result<()> {
                $(self.$idx.write(value.$idx)?;)+
                Ok(())
            }
        }
    };
}

tuples!(0 => A);
tuples!(0 => A, 1 => B);
tuples!(0 => A, 1 => B, 2 => C);
tuples!(0 => A, 1 => B, 2 => C, 3 => D);
tuples!(0 => A, 1 => B, 2 => C, 3 => D, 4 => E);
tuples!(0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F);
tuples!(0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G);
tuples!(0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H);
tuples!(0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H, 8 => I);
tuples!(0 => A, 1 => B, 2 => C, 3 => D, 4 => E, 5 => F, 6 => G, 7 => H, 8 => I, 9 => J);

pub struct Variant<W: ElementWriters, N: VariantName> {
    discriminators: Vec<u8>,
    writers: W,
    name: PhantomData<fn() -> N>,
}

impl<W: ElementWriters, N: VariantName> Variant<W, N> {
    pub const fn new(writers: W) -> Self {
        Self {
            discriminators: Vec::new(),
            writers,
            name: PhantomData,
        }
    }

    fn write_variant<'a>(&mut self, value: impl WriteVariant<'a, Writers = W>) -> Result<()> {
        let discriminator = value.write(&mut self.writers)?;
        self.discriminators.push(discriminator);
        Ok(())
    }
}

impl<W: ElementWriters + Default, N: VariantName> Default for Variant<W, N> {
    fn default() -> Self {
        Self::new(W::default())
    }
}

impl<W: ElementWriters, N: VariantName> ColumnWriter for Variant<W, N> {
    /// Panics unless the elements are declared in ClickHouse's order (sorted by type name),
    /// as the discriminators would otherwise name the wrong elements.
    fn write_type_name(&self, out: &mut String) {
        if let Some(name) = N::TYPE_NAME {
            out.push_str(name);
            return;
        }
        let mut names = Vec::new();
        self.writers.write_type_names(&mut names);
        assert!(
            names.is_sorted(),
            "declare enum variants sorted by ClickHouse type name, not as Variant({})",
            names.join(", ")
        );
        out.push_str("Variant(");
        out.push_str(&names.join(","));
        out.push(')');
    }

    fn write_prefix(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(&0u64.to_le_bytes())?;
        self.writers.write_prefix(out)
    }

    fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(&self.discriminators)?;
        self.writers.write_data(out)
    }

    fn write_default(&mut self) {
        self.discriminators
            .push(crate::mark::Variant::NULL_DISCRIMINATOR);
    }

    fn flush(&mut self) -> Result<()> {
        self.writers.flush()
    }

    fn reserve(&mut self, rows: usize) {
        self.discriminators.reserve(rows);
        self.writers.reserve(rows);
    }

    fn clear(&mut self) {
        self.discriminators.clear();
        self.writers.clear();
    }
}

impl<'a, T: WriteVariant<'a> + 'a> Writable<'a> for reader::Variant<'a, T> {
    type Writer = Variant<T::Writers, T::Name>;
}

impl<'a, T: WriteVariant<'a> + 'a> WriteColumn<'a, reader::Variant<'a, T>>
    for Variant<T::Writers, T::Name>
{
    fn for_reader(reader: &reader::Variant<'a, T>) -> Self {
        Self::new(T::writers(&reader.readers))
    }

    fn write_item(&mut self, item: T) -> Result<()> {
        self.write_variant(item)
    }
}

impl<'a, T: WriteVariant<'a> + 'a> Writable<'a> for reader::VariantNullable<'a, T> {
    type Writer = Variant<T::Writers, T::Name>;
}

impl<'a, T: WriteVariant<'a> + 'a> WriteColumn<'a, reader::VariantNullable<'a, T>>
    for Variant<T::Writers, T::Name>
{
    fn for_reader(reader: &reader::VariantNullable<'a, T>) -> Self {
        Self::new(T::writers(&reader.inner.readers))
    }

    fn write_item(&mut self, item: Option<T>) -> Result<()> {
        match item {
            Some(value) => self.write_variant(value),
            None => {
                self.write_default();
                Ok(())
            }
        }
    }
}

impl<W: ElementWriters, N: VariantName> WriteValue for Variant<W, N>
where
    for<'a> N::Value<'a>: WriteVariant<'a, Writers = W>,
{
    type Value<'a> = Option<N::Value<'a>>;

    fn write(&mut self, value: Option<N::Value<'_>>) -> Result<()> {
        match value {
            Some(value) => self.write_variant(value),
            None => {
                self.write_default();
                Ok(())
            }
        }
    }
}
