use std::fmt::Write as _;
use std::io::{self, Write};

use bstr::BStr;

use super::{
    ColumnWriter, ConstSizeType, ConstSizeWriter, Writable, WriteColumn, WriteValue, write_string,
};
use crate::{Error, Result, reader};

macro_rules! strings {
    ($($writer:ident: $value:ty => [$($reader:ident: $item:ty),+];)+) => {$(
        #[derive(Default)]
        pub struct $writer {
            data: Vec<u8>,
        }

        impl ColumnWriter for $writer {
            fn write_type_name(&self, out: &mut String) {
                out.push_str("String");
            }

            fn write_prefix(&self, _out: &mut dyn Write) -> io::Result<()> {
                Ok(())
            }

            fn write_data(&self, out: &mut dyn Write) -> io::Result<()> {
                out.write_all(&self.data)
            }

            fn write_default(&mut self) {
                self.data.push(0);
            }

            fn reserve(&mut self, rows: usize) {
                self.data.reserve(rows);
            }

            fn clear(&mut self) {
                self.data.clear();
            }
        }

        impl WriteValue for $writer {
            type Value<'a> = $value;

            fn write(&mut self, value: Self::Value<'_>) -> Result<()> {
                write_string(value.as_ref(), &mut self.data);
                Ok(())
            }
        }

        $(
            impl<'a> Writable<'a> for reader::$reader<'a> {
                type Writer = $writer;
            }

            impl<'a> WriteColumn<'a, reader::$reader<'a>> for $writer {
                fn for_reader(_reader: &reader::$reader<'a>) -> Self {
                    Self::default()
                }

                fn write_item(&mut self, item: $item) -> Result<()> {
                    self.write(item.as_ref())
                }
            }
        )+
    )+};
}

strings! {
    Str: &'a str => [Str: &'a str, TrustedStr: &'a str];
    Bytes: &'a [u8] => [Bytes: &'a BStr];
}

macro_rules! fixed_strings {
    ($($ty:ident: $value:ty => [$($reader:ident: $item:ty),+];)+) => {$(
        #[derive(Clone, Copy)]
        pub struct $ty {
            pub size: usize,
        }

        impl ConstSizeType for $ty {
            type Value<'a> = $value;

            fn write_type_name(&self, out: &mut String) {
                let _ = write!(out, "FixedString({})", self.size);
            }

            fn value_size(&self) -> usize {
                self.size
            }

            fn write(&self, value: Self::Value<'_>, data: &mut Vec<u8>) -> Result<()> {
                let bytes: &[u8] = value.as_ref();
                if bytes.len() > self.size {
                    return Err(Error::FixedStringOverflow(bytes.len(), self.size));
                }
                data.extend_from_slice(bytes);
                data.resize(data.len() + self.size - bytes.len(), 0);
                Ok(())
            }
        }

        $(
            impl<'a> Writable<'a> for reader::$reader<'a> {
                type Writer = ConstSizeWriter<$ty>;
            }

            impl<'a> WriteColumn<'a, reader::$reader<'a>> for ConstSizeWriter<$ty> {
                fn for_reader(reader: &reader::$reader<'a>) -> Self {
                    let size = reader.0.size;
                    Self::new($ty { size })
                }

                fn write_item(&mut self, item: $item) -> Result<()> {
                    self.write(item.as_ref())
                }
            }
        )+
    )+};
}

fixed_strings! {
    FixedStr: &'a str => [FixedStr: &'a str, TrustedFixedStr: &'a str];
    FixedBytes: &'a [u8] => [FixedBytes: &'a BStr];
}
