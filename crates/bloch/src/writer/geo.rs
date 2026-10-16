use std::io::{self, Write};

use super::{Array, ColumnWriter, ConstSizeWriter, F64, Writable, WriteColumn, WriteValue};
use crate::reader::TryRead;
use crate::{Result, reader};

macro_rules! geo {
    ($($name:ident($writer:ty) for $reader:ty => $spelling:literal;)+) => {$(
        #[doc = concat!("`", $spelling, "`, stored as `", stringify!($writer), "`.")]
        #[derive(Default)]
        pub struct $name(pub $writer);

        impl ColumnWriter for $name {
            fn write_type_name(&self, out: &mut String) {
                out.push_str($spelling);
            }

            fn write_prefix(&self, out: &mut dyn Write) -> io::Result<()> {
                ColumnWriter::write_prefix(&self.0, out)
            }

            fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
                ColumnWriter::write_data(&self.0, out)
            }

            fn write_default(&mut self) {
                ColumnWriter::write_default(&mut self.0);
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

        impl WriteValue for $name {
            type Value<'a> = <$writer as WriteValue>::Value<'a>;

            fn write(&mut self, value: Self::Value<'_>) -> Result<()> {
                self.0.write(value)
            }
        }

        impl<'a> Writable<'a> for reader::$name<'a> {
            type Writer = $name;
        }

        impl<'a> WriteColumn<'a, reader::$name<'a>> for $name {
            fn for_reader(reader: &reader::$name<'a>) -> Self {
                Self(<$writer as WriteColumn<'a, $reader>>::for_reader(&reader.0))
            }

            fn write_item(&mut self, item: <$reader as TryRead<'a>>::Item) -> Result<()> {
                <$writer as WriteColumn<'a, $reader>>::write_item(&mut self.0, item)
            }
        }
    )+};
}

geo! {
    Point((ConstSizeWriter<F64>, ConstSizeWriter<F64>)) for reader::Tuple<(reader::F64<'a>, reader::F64<'a>)> => "Point";
    Ring(Array<Point>) for reader::Array<'a, reader::Point<'a>> => "Ring";
    Polygon(Array<Ring>) for reader::Array<'a, reader::Ring<'a>> => "Polygon";
    MultiPolygon(Array<Polygon>) for reader::Array<'a, reader::Polygon<'a>> => "MultiPolygon";
    LineString(Array<Point>) for reader::Array<'a, reader::Point<'a>> => "LineString";
    MultiLineString(Array<LineString>) for reader::Array<'a, reader::LineString<'a>> => "MultiLineString";
    MultiPoint(Array<Point>) for reader::Array<'a, reader::Point<'a>> => "MultiPoint";
}
