//! Rewrite borrowed, derive-generated items into a standalone Native block.
//! Run with an optional output filename, then read it with ClickHouse:
//! cargo run -p bloch-examples --example write_lc_map -- /tmp/lc-map.native
//! clickhouse-local --input-format Native --file /tmp/lc-map.native \
//!   --query 'SELECT name, resource_attrs, attrs FROM table FORMAT PrettyCompact'

use bloch::parse::block::parse_single;
use bloch::reader::{LcStr, Map};
use bloch::{BlockWriter, FromBlock, WriteBlock};

#[derive(FromBlock, WriteBlock, Copy, Clone)]
struct Row<'a> {
    name: LcStr<'a>,
    #[col(name = "resource_attrs")]
    attributes: Map<'a, LcStr<'a>, LcStr<'a>>,
    attrs: Map<'a, LcStr<'a>, LcStr<'a>>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let source = include_bytes!("../../bloch/testdata/activity_hw.native");
    let (remaining, block) = parse_single(source)?;
    assert!(remaining.is_empty());

    // WriteBlock implements WriteRow for the generated RowItem. Each write consumes the lazy
    // maps directly; nothing is collected in between.
    let mut writer = BlockWriter::with_capacity(block.num_rows);
    for item in Row::rows(&block)? {
        let mut item = item?;
        item.name = "rewritten";
        writer.write(item)?;
    }
    let mut native = Vec::new();
    writer.encode(&mut native)?;

    let (remaining, output) = parse_single(&native)?;
    assert!(remaining.is_empty());
    assert_eq!(output.num_rows, block.num_rows);
    for (original, rewritten) in Row::rows(&block)?.zip(Row::rows(&output)?) {
        let original = original?;
        let rewritten = rewritten?;
        assert_eq!(rewritten.name, "rewritten");
        assert_eq!(
            original.attributes.collect::<bloch::Result<Vec<_>>>()?,
            rewritten.attributes.collect::<bloch::Result<Vec<_>>>()?,
        );
        assert_eq!(
            original.attrs.collect::<bloch::Result<Vec<_>>>()?,
            rewritten.attrs.collect::<bloch::Result<Vec<_>>>()?,
        );
    }

    if let Some(path) = std::env::args_os().nth(1) {
        std::fs::write(path, &native)?;
    }
    println!(
        "Wrote and read back {} rows, {} columns, {} Native bytes",
        output.num_rows,
        output.col_names.len(),
        native.len(),
    );
    Ok(())
}
