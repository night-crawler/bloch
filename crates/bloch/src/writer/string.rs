use std::fmt::Write as _;
use std::io::{self, Write};

use bstr::BStr;

use super::{ColumnWriter, ConstSizeType, ConstSizeWriter, Writable, WriteColumn, write_string};
use crate::{Error, Result, reader};

#[derive(Default)]
pub struct Str {
    data: Vec<u8>,
}

impl ColumnWriter for Str {
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

macro_rules! strings {
    ($($reader:ident: $item:ty;)+) => {$(
        impl<'a> Writable<'a> for reader::$reader<'a> {
            type Writer = Str;
        }

        impl<'a> WriteColumn<'a, reader::$reader<'a>> for Str {
            fn for_reader(_reader: &reader::$reader<'a>) -> Self {
                Self::default()
            }

            fn write(&mut self, item: $item) -> Result<()> {
                write_string(item.as_ref(), &mut self.data);
                Ok(())
            }
        }
    )+};
}

strings! {
    Str: &'a str;
    TrustedStr: &'a str;
    Bytes: &'a BStr;
}

#[derive(Clone, Copy)]
pub struct FixedStr {
    pub size: usize,
}

impl ConstSizeType for FixedStr {
    fn write_type_name(&self, out: &mut String) {
        let _ = write!(out, "FixedString({})", self.size);
    }

    fn value_size(&self) -> usize {
        self.size
    }
}

impl ConstSizeWriter<FixedStr> {
    fn push(&mut self, value: &[u8]) -> Result<()> {
        let size = self.ty.size;
        if value.len() > size {
            return Err(Error::FixedStringOverflow(value.len(), size));
        }
        self.data.extend_from_slice(value);
        self.data.resize(self.data.len() + size - value.len(), 0);
        Ok(())
    }
}

macro_rules! fixed_strings {
    ($($reader:ident: $item:ty;)+) => {$(
        impl<'a> Writable<'a> for reader::$reader<'a> {
            type Writer = ConstSizeWriter<FixedStr>;
        }

        impl<'a> WriteColumn<'a, reader::$reader<'a>> for ConstSizeWriter<FixedStr> {
            fn for_reader(reader: &reader::$reader<'a>) -> Self {
                let size = reader.0.size;
                Self::new(FixedStr { size })
            }

            fn write(&mut self, item: $item) -> Result<()> {
                self.push(item.as_ref())
            }
        }
    )+};
}

fixed_strings! {
    FixedStr: &'a str;
    TrustedFixedStr: &'a str;
    FixedBytes: &'a BStr;
}
