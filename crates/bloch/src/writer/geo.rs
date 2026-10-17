use std::borrow::Cow;
use std::io::{self, Write};

use super::{Array, ColumnWriter, DefaultWriter, F64, FixedWidthColumn, FromReader, ValueWriter};
use crate::reader::TryRead;
use crate::{Result, reader};

macro_rules! geo {
    ($($name:ident($writer:ty) for $reader:ty => $spelling:literal;)+) => {$(
        #[doc = concat!("`", $spelling, "`, stored as `", stringify!($writer), "`.")]
        #[derive(Default)]
        pub struct $name(pub $writer);

        impl ColumnWriter for $name {
            fn type_name(&self) -> Cow<'static, str> {
                $spelling.into()
            }

            fn write_prefix(&self, out: &mut dyn Write) -> io::Result<()> {
                ColumnWriter::write_prefix(&self.0, out)
            }

            fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
                ColumnWriter::write_data(&self.0, out)
            }

            fn push_default(&mut self) {
                ColumnWriter::push_default(&mut self.0);
            }

            fn reserve(&mut self, rows: usize) {
                ColumnWriter::reserve(&mut self.0, rows);
            }

            fn clear(&mut self) {
                ColumnWriter::clear(&mut self.0);
            }

            fn flush(&mut self) -> crate::Result<()> {
                ColumnWriter::flush(&mut self.0)
            }
        }

        impl ValueWriter for $name {
            type Value<'a> = <$writer as ValueWriter>::Value<'a>;

            fn write(&mut self, value: Self::Value<'_>) -> Result<()> {
                self.0.write(value)
            }
        }

        impl<'a> DefaultWriter<'a> for reader::$name<'a> {
            type Writer = $name;
        }

        impl<'a> FromReader<'a, reader::$name<'a>> for $name {
            fn from_reader(reader: &reader::$name<'a>) -> Self {
                Self(<$writer as FromReader<'a, $reader>>::from_reader(&reader.0))
            }

            fn write_item(&mut self, item: <$reader as TryRead<'a>>::Item) -> Result<()> {
                <$writer as FromReader<'a, $reader>>::write_item(&mut self.0, item)
            }
        }
    )+};
}

geo! {
    Point((FixedWidthColumn<F64>, FixedWidthColumn<F64>)) for reader::Tuple<(reader::F64<'a>, reader::F64<'a>)> => "Point";
    Ring(Array<Point>) for reader::Array<'a, reader::Point<'a>> => "Ring";
    Polygon(Array<Ring>) for reader::Array<'a, reader::Ring<'a>> => "Polygon";
    MultiPolygon(Array<Polygon>) for reader::Array<'a, reader::Polygon<'a>> => "MultiPolygon";
    LineString(Array<Point>) for reader::Array<'a, reader::Point<'a>> => "LineString";
    MultiLineString(Array<LineString>) for reader::Array<'a, reader::LineString<'a>> => "MultiLineString";
    MultiPoint(Array<Point>) for reader::Array<'a, reader::Point<'a>> => "MultiPoint";
}
