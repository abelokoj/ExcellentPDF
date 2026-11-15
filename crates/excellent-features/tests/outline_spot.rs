//! B.10 on a real file: an entry made from a bookmark's place (page, point, zoom) reads back with the same title and
//! the same place, so turning it back into a bookmark loses nothing.

use std::path::Path;

use excellent_features::outline::{self, Spot};
use excellent_mupdf::{Document, NewOutlineEntry, PagePoint};

#[test]
fn a_place_survives_the_trip_into_the_outline_and_back() {
    let file =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus/base14-outline-annot.pdf");
    let mut doc = Document::open_pdf(&file).expect("open");
    let places = [
        (
            1,
            Spot {
                x: 30.0,
                y: 120.0,
                zoom: Some(1.5),
            },
        ),
        (
            0,
            Spot {
                x: 0.0,
                y: 64.0,
                zoom: None,
            },
        ),
    ];
    for (n, (page, spot)) in places.iter().enumerate() {
        let uri = doc
            .link_uri(
                *page,
                PagePoint {
                    x: spot.x,
                    y: spot.y,
                },
                spot.zoom,
            )
            .expect("uri");
        let title = format!("Kept {n}");
        doc.operation("Add outline entry", |edit| {
            edit.insert_outline_entry(
                &[0],
                NewOutlineEntry {
                    title: &title,
                    uri: &uri,
                },
            )
        })
        .expect("insert");
        let items = doc.outline().expect("outline");
        let entry = &items[0];
        assert_eq!(entry.title, title);
        assert_eq!(entry.page, Some(*page), "{entry:?}");
        assert_eq!(outline::spot(entry.uri.as_deref()), *spot, "{entry:?}");
    }
}
