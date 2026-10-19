use std::{
    borrow::Cow,
    io::{self, Write},
};

use super::{
    ColumnWriter, DefaultWriter, FromReader, IntoVariant, ValueWriter, VariantValue, write_offsets,
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
    fn type_name(&self) -> Cow<'static, str> {
        format!("Nullable({})", self.inner.type_name()).into()
    }

    fn write_prefix(&self, out: &mut dyn Write) -> io::Result<()> {
        self.inner.write_prefix(out)
    }

    fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(&self.nulls)?;
        self.inner.write_data(out)
    }

    fn data_len(&self) -> usize {
        self.nulls.len() + self.inner.data_len()
    }

    fn push_default(&mut self) {
        self.nulls.push(1);
        self.inner.push_default();
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

impl<'a, Inner: DefaultWriter<'a> + 'a> DefaultWriter<'a> for reader::Nullable<'a, Inner> {
    type Writer = Nullable<Inner::Writer>;
}

impl<'a, Inner: DefaultWriter<'a> + 'a> FromReader<'a, reader::Nullable<'a, Inner>>
    for Nullable<Inner::Writer>
{
    fn from_reader(reader: &reader::Nullable<'a, Inner>) -> Self {
        Self::new(Inner::Writer::from_reader(&reader.inner))
    }

    fn write_item(&mut self, item: Option<Inner::Item>) -> Result<()> {
        match item {
            Some(value) => {
                self.nulls.push(0);
                self.inner.write_item(value)
            }
            None => {
                self.push_default();
                Ok(())
            }
        }
    }
}

impl<Inner: ValueWriter> ValueWriter for Nullable<Inner> {
    type Value<'a> = Option<Inner::Value<'a>>;

    fn write(&mut self, value: Option<Inner::Value<'_>>) -> Result<()> {
        match value {
            Some(value) => {
                self.nulls.push(0);
                self.inner.write(value)
            }
            None => {
                self.push_default();
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
        Values: ValueWriter,
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
    fn type_name(&self) -> Cow<'static, str> {
        format!("Array({})", self.values.type_name()).into()
    }

    fn write_prefix(&self, out: &mut dyn Write) -> io::Result<()> {
        self.values.write_prefix(out)
    }

    fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
        write_offsets(&self.offsets, out)?;
        self.values.write_data(out)
    }

    fn data_len(&self) -> usize {
        self.offsets.len() * size_of::<u64>() + self.values.data_len()
    }

    fn push_default(&mut self) {
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

impl<'a, Inner: DefaultWriter<'a> + 'a> DefaultWriter<'a> for reader::Array<'a, Inner> {
    type Writer = Array<Inner::Writer>;
}

impl<'a, Inner: DefaultWriter<'a> + 'a> FromReader<'a, reader::Array<'a, Inner>>
    for Array<Inner::Writer>
{
    fn from_reader(reader: &reader::Array<'a, Inner>) -> Self {
        let mut values = Inner::Writer::from_reader(&reader.values);
        values.reserve(reader.values.len());
        Self::new(values)
    }

    fn write_item(&mut self, item: reader::ArrayIter<'a, Inner>) -> Result<()> {
        self.write_with(item, |values, item| values.write_item(item?))
    }
}

impl<Values> ValueWriter for Array<Values>
where
    Values: ValueWriter,
    for<'a> Values::Value<'a>: Copy,
{
    type Value<'a> = &'a [Values::Value<'a>];

    #[inline(always)]
    fn write(&mut self, values: Self::Value<'_>) -> Result<()> {
        self.values.write_slice(values)?;
        self.offsets
            .push(self.offsets.last().copied().unwrap_or(0) + values.len() as u64);
        Ok(())
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
    fn type_name(&self) -> Cow<'static, str> {
        format!("Map({},{})", self.keys.type_name(), self.values.type_name()).into()
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

    fn data_len(&self) -> usize {
        self.offsets.len() * size_of::<u64>() + self.keys.data_len() + self.values.data_len()
    }

    fn push_default(&mut self) {
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

impl<'a, K, V> DefaultWriter<'a> for reader::Map<'a, K, V>
where
    K: DefaultWriter<'a> + 'a,
    V: DefaultWriter<'a> + 'a,
{
    type Writer = Map<K::Writer, V::Writer>;
}

impl<'a, K, V> FromReader<'a, reader::Map<'a, K, V>> for Map<K::Writer, V::Writer>
where
    K: DefaultWriter<'a> + 'a,
    V: DefaultWriter<'a> + 'a,
{
    fn from_reader(reader: &reader::Map<'a, K, V>) -> Self {
        let mut keys = K::Writer::from_reader(&reader.keys);
        keys.reserve(reader.keys.len());
        let mut values = V::Writer::from_reader(&reader.values);
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

impl<Keys, Values> ValueWriter for Map<Keys, Values>
where
    Keys: ValueWriter,
    Values: ValueWriter,
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
        impl<$($t: ColumnWriter,)+> ColumnWriter for ($($t,)+) {
            fn type_name(&self) -> Cow<'static, str> {
                let names = [$(self.$idx.type_name(),)+];
                format!("Tuple({})", names.join(",")).into()
            }
            fn write_prefix(&self, out: &mut dyn Write) -> io::Result<()> {
                $(self.$idx.write_prefix(out)?;)+
                Ok(())
            }

            fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
                $(self.$idx.write_data(out)?;)+
                Ok(())
            }

            fn data_len(&self) -> usize {
                0 $(+ self.$idx.data_len())+
            }

            fn push_default(&mut self) {
                $(self.$idx.push_default();)+
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

        impl<'a, $($t: DefaultWriter<'a> + 'a,)+> DefaultWriter<'a> for reader::Tuple<($($t,)+)> {
            type Writer = ($($t::Writer,)+);
        }

        impl<'a, $($t: DefaultWriter<'a> + 'a,)+> FromReader<'a, reader::Tuple<($($t,)+)>>
            for ($($t::Writer,)+)
        {
            fn from_reader(reader: &reader::Tuple<($($t,)+)>) -> Self {
                ($($t::Writer::from_reader(&reader.0.$idx),)+)
            }

            fn write_item(&mut self, item: ($($t::Item,)+)) -> Result<()> {
                $(self.$idx.write_item(item.$idx)?;)+
                Ok(())
            }
        }

        impl<$($t: ValueWriter,)+> ValueWriter for ($($t,)+) {
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

pub struct Variant<V: VariantValue> {
    discriminators: Vec<u8>,
    columns: V::Columns,
}

impl<V: VariantValue> Variant<V> {
    pub const fn new(columns: V::Columns) -> Self {
        Self {
            discriminators: Vec::new(),
            columns,
        }
    }

    fn push<'a>(&mut self, value: impl IntoVariant<'a, Marker = V>) -> Result<()> {
        let discriminator = value.write(&mut self.columns)?;
        self.discriminators.push(discriminator);
        Ok(())
    }
}

impl<V: VariantValue> Default for Variant<V>
where
    V::Columns: Default,
{
    fn default() -> Self {
        Self::new(V::Columns::default())
    }
}

impl<V: VariantValue> ColumnWriter for Variant<V> {
    /// Panics unless the elements are declared in ClickHouse's order (sorted by type name),
    /// as the discriminators would otherwise name the wrong elements.
    fn type_name(&self) -> Cow<'static, str> {
        if let Some(name) = V::TYPE_NAME {
            return name.into();
        }
        let names = V::type_names(&self.columns);
        assert!(
            names.is_sorted(),
            "declare enum variants sorted by ClickHouse type name, not as Variant({})",
            names.join(", ")
        );
        format!("Variant({})", names.join(",")).into()
    }

    fn write_prefix(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(&0u64.to_le_bytes())?;
        self.columns.write_prefix(out)
    }

    fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
        out.write_all(&self.discriminators)?;
        self.columns.write_data(out)
    }

    fn data_len(&self) -> usize {
        self.discriminators.len() + self.columns.data_len()
    }

    fn push_default(&mut self) {
        self.discriminators
            .push(crate::mark::Variant::NULL_DISCRIMINATOR);
    }

    fn flush(&mut self) -> Result<()> {
        self.columns.flush()
    }

    fn reserve(&mut self, rows: usize) {
        self.discriminators.reserve(rows);
        self.columns.reserve(rows);
    }

    fn clear(&mut self) {
        self.discriminators.clear();
        self.columns.clear();
    }
}

impl<'a, T: IntoVariant<'a> + 'a> DefaultWriter<'a> for reader::Variant<'a, T> {
    type Writer = Variant<T::Marker>;
}

impl<'a, T: IntoVariant<'a> + 'a> FromReader<'a, reader::Variant<'a, T>> for Variant<T::Marker> {
    fn from_reader(reader: &reader::Variant<'a, T>) -> Self {
        Self::new(T::columns(&reader.readers))
    }

    fn write_item(&mut self, item: T) -> Result<()> {
        self.push(item)
    }
}

impl<'a, T: IntoVariant<'a> + 'a> DefaultWriter<'a> for reader::VariantNullable<'a, T> {
    type Writer = Variant<T::Marker>;
}

impl<'a, T: IntoVariant<'a> + 'a> FromReader<'a, reader::VariantNullable<'a, T>>
    for Variant<T::Marker>
{
    fn from_reader(reader: &reader::VariantNullable<'a, T>) -> Self {
        Self::new(T::columns(&reader.inner.readers))
    }

    fn write_item(&mut self, item: Option<T>) -> Result<()> {
        match item {
            Some(value) => self.push(value),
            None => {
                self.push_default();
                Ok(())
            }
        }
    }
}

impl<V: VariantValue> ValueWriter for Variant<V>
where
    for<'a> V::Value<'a>: IntoVariant<'a, Marker = V>,
{
    type Value<'a> = Option<V::Value<'a>>;

    fn write(&mut self, value: Option<V::Value<'_>>) -> Result<()> {
        match value {
            Some(value) => self.push(value),
            None => {
                self.push_default();
                Ok(())
            }
        }
    }
}
