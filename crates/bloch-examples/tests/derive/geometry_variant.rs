use bloch::FromBlock;
use bloch::parse::block::parse_single;
use bloch::reader::{Array, ArrayIter, Geometry, I64, Point, Ring, TryRead, VariantNullable};

const _SQL: &str = r#"
-- Geometry / MultiPoint: MultiPoint is unknown to ClickHouse 26.3 and older
-- (including 25.3), so both the server and the client must be 26.8+;
-- reproduced on ClickHouse 26.8.2.7.

drop table if exists geometry_sample;

create table geometry_sample
(
    id  Int64,
    geo Geometry,
    mp  MultiPoint,
    arr Array(Geometry)
) engine = MergeTree order by id;

insert into geometry_sample values
    (1, readWKT('POINT(1 2)'), readWKT('MULTIPOINT(1 1,2 2,3 3)'), [readWKT('POINT(1 2)'), NULL]),
    (2, readWKT('LINESTRING(0 0,1 1,2 0)'), [], []),
    (3, readWKT('MULTILINESTRING((0 0,1 1),(2 2,3 3,4 2))'), [(7, 7)], [readWKT('LINESTRING(0 0,1 1)')]),
    (4, readWKT('POLYGON((0 0,10 0,10 10,0 10,0 0),(4 4,5 4,5 5,4 5,4 4))'), [], [NULL]),
    (5, readWKT('MULTIPOLYGON(((0 0,1 0,1 1,0 0)),((5 5,6 5,6 6,5 5),(5.2 5.2,5.5 5.2,5.5 5.5,5.2 5.2)))'), [], []),
    (6, CAST([(0, 0), (1, 0), (1, 1)], 'Ring'), [], []),
    (7, readWKT('MULTIPOINT(1 1,2 2,3 3)'), [], [readWKT('MULTIPOINT(9 9)')]),
    (8, NULL, [], []);

optimize table geometry_sample final;

select id, geo, mp, arr from geometry_sample order by id format Native;
"#;

#[derive(FromBlock, Copy, Clone)]
struct Row<'a> {
    id: I64<'a>,
    // Variant is implicitly nullable, and so is Geometry.
    geo: VariantNullable<'a, Geometry<'a>>,
    mp: Ring<'a>,
    arr: Array<'a, VariantNullable<'a, Geometry<'a>>>,
}

fn ring<'a>(points: ArrayIter<'a, Point<'a>>) -> bloch::Result<Vec<(f64, f64)>> {
    points.try_collect_vec()
}

/// Rings of a `Polygon` or lines of a `MultiLineString`: both are arrays of point arrays.
fn polygon<'a, Line>(rings: ArrayIter<'a, Line>) -> bloch::Result<Vec<Vec<(f64, f64)>>>
where
    Line: TryRead<'a, Item = ArrayIter<'a, Point<'a>>>,
{
    rings.map(|r| ring(r?)).collect()
}

#[test]
fn reads_geometry_by_discriminator_order() -> Result<(), Box<dyn std::error::Error>> {
    let data = std::fs::read(crate::common::fixture("geometry_sample.native"))?;
    let (_, block) = parse_single(&data)?;
    let mut rows = 0;
    for (index, row) in Row::rows(&block)?.enumerate() {
        let row = row?;
        assert_eq!(row.id, i64::try_from(index + 1)?);
        match (row.id, row.geo) {
            (1, Some(Geometry::Point(p))) => assert_eq!(p, (1.0, 2.0)),
            (2, Some(Geometry::LineString(l))) => {
                assert_eq!(ring(l)?, [(0.0, 0.0), (1.0, 1.0), (2.0, 0.0)]);
            }
            (3, Some(Geometry::MultiLineString(ml))) => assert_eq!(
                polygon(ml)?,
                [
                    vec![(0.0, 0.0), (1.0, 1.0)],
                    vec![(2.0, 2.0), (3.0, 3.0), (4.0, 2.0)],
                ]
            ),
            (4, Some(Geometry::Polygon(p))) => assert_eq!(
                polygon(p)?,
                [
                    vec![
                        (0.0, 0.0),
                        (10.0, 0.0),
                        (10.0, 10.0),
                        (0.0, 10.0),
                        (0.0, 0.0)
                    ],
                    vec![(4.0, 4.0), (5.0, 4.0), (5.0, 5.0), (4.0, 5.0), (4.0, 4.0)],
                ]
            ),
            (5, Some(Geometry::MultiPolygon(mp))) => {
                let polygons: Vec<_> = mp.map(|p| polygon(p?)).collect::<bloch::Result<_>>()?;
                assert_eq!(polygons.len(), 2);
                assert_eq!(
                    polygons[0],
                    [vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 0.0)]]
                );
                assert_eq!(
                    polygons[1],
                    [
                        vec![(5.0, 5.0), (6.0, 5.0), (6.0, 6.0), (5.0, 5.0)],
                        vec![(5.2, 5.2), (5.5, 5.2), (5.5, 5.5), (5.2, 5.2)],
                    ]
                );
            }
            (6, Some(Geometry::Ring(r))) => {
                assert_eq!(ring(r)?, [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)])
            }
            (7, Some(Geometry::MultiPoint(mp))) => {
                assert_eq!(ring(mp)?, [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0)]);
            }
            (8, None) => {}
            (id, _) => panic!("unexpected geometry variant at id {id}"),
        }

        let mp = row.mp.try_collect_vec()?;
        match row.id {
            1 => assert_eq!(mp, [(1.0, 1.0), (2.0, 2.0), (3.0, 3.0)]),
            3 => assert_eq!(mp, [(7.0, 7.0)]),
            _ => assert!(mp.is_empty()),
        }

        let mut arr = row.arr.try_collect_vec()?;
        match (row.id, arr.len()) {
            (1, 2) => {
                assert!(arr.pop().flatten().is_none());
                let Some(Some(Geometry::Point(p))) = arr.pop() else {
                    panic!("expected [Point, NULL] at id 1");
                };
                assert_eq!(p, (1.0, 2.0));
            }
            (3, 1) => {
                let Some(Some(Geometry::LineString(l))) = arr.pop() else {
                    panic!("expected [LineString] at id 3");
                };
                assert_eq!(ring(l)?, [(0.0, 0.0), (1.0, 1.0)]);
            }
            (4, 1) => assert!(arr.pop().flatten().is_none()),
            (7, 1) => {
                let Some(Some(Geometry::MultiPoint(mp))) = arr.pop() else {
                    panic!("expected [MultiPoint] at id 7");
                };
                assert_eq!(ring(mp)?, [(9.0, 9.0)]);
            }
            (_, 0) => {}
            (id, len) => panic!("unexpected arr of length {len} at id {id}"),
        }
        rows += 1;
    }
    assert_eq!(rows, 8);
    Ok(())
}
