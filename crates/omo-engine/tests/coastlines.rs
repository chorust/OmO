//! `OCB1` header/index parser, bbox culling and Cohen–Sutherland clipping tests against the
//! shipped `fixtures/geodata/ne_50m_coast_borders.ocb1` (`T031`, `SC-005`).

use omo_protocol::coastlines::{FeatureKind, Ocb1, Ocb1Error, builtin, clip_segment, polylines};

fn fixture() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/geodata/ne_50m_coast_borders.ocb1"
    ))
    .expect("fixture present")
}

#[test]
fn parses_the_shipped_asset_header_and_index() {
    let bytes = fixture();
    let asset = Ocb1::parse(&bytes).unwrap();
    assert!(asset.line_count() > 1000, "{}", asset.line_count());
    assert!(asset.vertex_count() as usize > asset.line_count());
    let mut coast = 0;
    let mut border = 0;
    for i in 0..asset.line_count() {
        match asset.kind(i) {
            FeatureKind::Coastline => coast += 1,
            FeatureKind::Admin0Border => border += 1,
        }
        let b = asset.line_bbox(i);
        assert!(b[0] <= b[2] && b[1] <= b[3]);
        let vertices = asset.vertices(i);
        assert!(vertices.len() >= 2);
        // The stored bbox bounds every decoded vertex (quantization is exact in 1e-4 deg).
        for (lon, lat) in vertices {
            assert!(lon >= b[0] - 1e-9 && lon <= b[2] + 1e-9, "line {i}");
            assert!(lat >= b[1] - 1e-9 && lat <= b[3] + 1e-9, "line {i}");
        }
    }
    assert!(coast > 0 && border > 0, "coast {coast} border {border}");
    assert_eq!(builtin().line_count(), asset.line_count());
}

#[test]
fn rejects_bad_magic_version_and_truncation() {
    let bytes = fixture();
    let mut bad_magic = bytes.clone();
    bad_magic[0] = b'X';
    assert_eq!(Ocb1::parse(&bad_magic).unwrap_err(), Ocb1Error::BadMagic);
    let mut bad_version = bytes.clone();
    bad_version[4] = 9;
    assert_eq!(
        Ocb1::parse(&bad_version).unwrap_err(),
        Ocb1Error::UnsupportedVersion(9)
    );
    assert!(matches!(
        Ocb1::parse(&bytes[..bytes.len() - 4]).unwrap_err(),
        Ocb1Error::Corrupt(_)
    ));
    assert_eq!(Ocb1::parse(&bytes[..8]).unwrap_err(), Ocb1Error::BadMagic);
}

#[test]
fn bbox_culling_keeps_only_intersecting_lines() {
    let asset = builtin();
    let europe = asset.lines_in_bbox([-10.0, 35.0, 30.0, 60.0]);
    assert!(!europe.is_empty());
    assert!(europe.len() * 5 < asset.line_count(), "culling ineffective");
    for index in europe {
        let b = asset.line_bbox(index);
        assert!(b[0] <= 30.0 && b[2] >= -10.0 && b[1] <= 60.0 && b[3] >= 35.0);
    }
    // The open South Pacific: at most the single long Americas coast passes the bbox test, and
    // segment clipping then yields nothing.
    let pacific = [-140.0, -50.0, -120.0, -40.0];
    assert!(asset.lines_in_bbox(pacific).len() <= 1);
    let mut segments = 0;
    asset.visit_segments(pacific, |_, _| segments += 1);
    assert_eq!(segments, 0);
}

#[test]
fn cohen_sutherland_clips_to_the_rectangle() {
    let rect = [0.0, 0.0, 10.0, 10.0];
    assert_eq!(
        clip_segment([2.0, 2.0, 8.0, 8.0], rect),
        Some([2.0, 2.0, 8.0, 8.0])
    );
    assert_eq!(clip_segment([-5.0, -5.0, -1.0, -1.0], rect), None);
    assert_eq!(clip_segment([-5.0, 12.0, 15.0, 14.0], rect), None);
    let crossing = clip_segment([-5.0, 5.0, 15.0, 5.0], rect).unwrap();
    assert_eq!(crossing, [0.0, 5.0, 10.0, 5.0]);
    let diagonal = clip_segment([-5.0, -5.0, 15.0, 15.0], rect).unwrap();
    for (a, b) in diagonal.iter().zip([0.0, 0.0, 10.0, 10.0]) {
        assert!((a - b).abs() < 1e-12, "{diagonal:?}");
    }
    // A segment that clips through two corners' neighbourhoods but misses the rectangle.
    assert_eq!(clip_segment([-5.0, 8.0, 3.0, 16.0], rect), None);
}

#[test]
fn clipped_segments_stay_inside_the_view() {
    let view = [100.0, 20.0, 125.0, 45.0];
    let mut count = 0usize;
    builtin().visit_segments(view, |_, [x0, y0, x1, y1]| {
        count += 1;
        for (x, y) in [(x0, y0), (x1, y1)] {
            assert!(x >= view[0] - 1e-9 && x <= view[2] + 1e-9, "{x}");
            assert!(y >= view[1] - 1e-9 && y <= view[3] + 1e-9, "{y}");
        }
    });
    assert!(
        count > 200,
        "East Asia should have many coast segments: {count}"
    );
}

#[test]
fn views_across_the_antimeridian_use_wrapped_copies() {
    // Wrangel Island sits just west of the antimeridian; its eastern half is at x > 180.
    let view = [170.0, 60.0, 190.0, 75.0];
    let mut beyond = 0usize;
    let mut inside = 0usize;
    builtin().visit_segments(view, |_, [x0, y0, x1, y1]| {
        for (x, y) in [(x0, y0), (x1, y1)] {
            assert!(x >= view[0] - 1e-9 && x <= view[2] + 1e-9);
            assert!(y >= view[1] - 1e-9 && y <= view[3] + 1e-9);
        }
        if x0 > 180.0 {
            beyond += 1;
        } else {
            inside += 1;
        }
    });
    assert!(inside > 0, "no segments west of the antimeridian");
    assert!(beyond > 0, "no wrapped segments beyond +180");
}

#[test]
fn views_wider_than_the_world_repeat_the_coastlines() {
    let mut xs = Vec::new();
    builtin().visit_segments([-540.0, -60.0, 540.0, 80.0], |_, s| xs.push(s[0]));
    let west = xs.iter().filter(|x| **x < -180.0).count();
    let mid = xs.iter().filter(|x| (-180.0..=180.0).contains(*x)).count();
    let east = xs.iter().filter(|x| **x > 180.0).count();
    assert!(west > 0 && mid > 0 && east > 0);
}

#[test]
fn polylines_do_not_bridge_coastline_and_border_across_shared_vertex() {
    // Build a minimal 3-line OCB1 asset:
    // Line 0 (Coastline): (0, 0) -> (10, 0)
    // Line 1 (Admin0Border): (10, 0) -> (10, 10)  (shares endpoint (10, 0) with Line 0!)
    // Line 2 (Coastline): (20, 20) -> (30, 20)    (disjoint from Line 0 and Line 1!)
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"OCB1");
    bytes.extend_from_slice(&1u16.to_le_bytes()); // version
    bytes.extend_from_slice(&0u16.to_le_bytes()); // flags
    bytes.extend_from_slice(&3u32.to_le_bytes()); // line_count = 3
    bytes.extend_from_slice(&6u32.to_le_bytes()); // vertex_count = 6
    let push_line =
        |buf: &mut Vec<u8>, x0: i32, y0: i32, x1: i32, y1: i32, delta_off: u32, kind: u8| {
            buf.extend_from_slice(&x0.min(x1).to_le_bytes());
            buf.extend_from_slice(&y0.min(y1).to_le_bytes());
            buf.extend_from_slice(&x0.max(x1).to_le_bytes());
            buf.extend_from_slice(&y0.max(y1).to_le_bytes());
            buf.extend_from_slice(&x0.to_le_bytes());
            buf.extend_from_slice(&y0.to_le_bytes());
            buf.extend_from_slice(&delta_off.to_le_bytes());
            buf.extend_from_slice(&2u16.to_le_bytes());
            buf.push(kind);
            buf.push(0);
        };
    let deg = |d: i32| d * 10_000;
    push_line(&mut bytes, deg(0), deg(0), deg(1), deg(0), 0, 0);
    push_line(&mut bytes, deg(1), deg(0), deg(1), deg(1), 1, 1);
    push_line(&mut bytes, deg(1), deg(1), deg(2), deg(1), 2, 0);
    for (dlon, dlat) in [(10_000i16, 0i16), (0, 10_000), (10_000, 0)] {
        bytes.extend_from_slice(&dlon.to_le_bytes());
        bytes.extend_from_slice(&dlat.to_le_bytes());
    }
    let asset = Ocb1::parse(&bytes).unwrap();
    let (coast, border) = polylines(&asset, [-5.0, -5.0, 10.0, 10.0], |lon, lat| {
        (lon * 100.0, lat * 100.0)
    });
    // Line 0 ends at (1, 0), Line 1 (border) goes (1, 0) -> (1, 1), Line 2 (coast) starts at (1, 1)!
    // With per-kind tracking, Line 0 and Line 2 must NOT bridge through Line 1's endpoint (1, 1).
    assert_eq!(
        coast.len(),
        2,
        "disjoint coast lines bridged via border: {coast:?}"
    );
    assert_eq!(border.len(), 1, "{border:?}");
}
