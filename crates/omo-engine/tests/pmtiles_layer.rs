//! Local `.pmtiles` v3 layer tests (`T032`, `SC-005`): raster decode, gzip and zstd MVT
//! rasterization, zoom/bounds clamping and overzoom, attribution, and error isolation.

use std::{io::Write, path::PathBuf};

use flate2::{Compression, write::GzEncoder};
use omo_engine::map::pmtiles_layer::{PmtilesError, PmtilesLayer, tile_id};
use omo_protocol::map::{PmtilesTileKind, ValueProvenance};
use sha2::{Digest, Sha256};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/pmtiles")
        .join(name)
}

fn px(tile: &[u8], u: usize, v: usize) -> [u8; 4] {
    let at = (v * 256 + u) * 4;
    [tile[at], tile[at + 1], tile[at + 2], tile[at + 3]]
}

/// Mirror of `tile_pixels` in `scripts/build-pmtiles-fixtures.py`.
fn expected_pixel(z: u8, x: u32, y: u32, u: usize, v: usize) -> [u8; 4] {
    if u < 2 || v < 2 || u >= 254 || v >= 254 {
        return [255, 255, 255, 255];
    }
    let h = Sha256::digest(format!("{z}/{x}/{y}").as_bytes());
    let a = (96 + (u + v) / 3).min(255) as u8;
    [60 + h[0] % 160, 60 + h[1] % 160, 60 + h[2] % 160, a]
}

#[test]
fn hilbert_tile_ids_match_the_pmtiles_spec() {
    assert_eq!(tile_id(0, 0, 0), 0);
    assert_eq!(tile_id(1, 0, 0), 1);
    assert_eq!(tile_id(1, 0, 1), 2);
    assert_eq!(tile_id(1, 1, 1), 3);
    assert_eq!(tile_id(1, 1, 0), 4);
    assert_eq!(tile_id(2, 0, 0), 5);
}

#[test]
fn raster_png_archive_decodes_exact_straight_rgba() {
    let layer = PmtilesLayer::open(&fixture("sample-raster.pmtiles")).unwrap();
    let info = layer.info(false);
    assert_eq!(info.tile_kind, PmtilesTileKind::RasterPng);
    assert_eq!((info.min_zoom, info.max_zoom), (0, 3));
    assert_eq!(info.value_provenance, ValueProvenance::DisplayOnly);
    assert!(info.bounds[0] <= -179.9 && info.bounds[2] >= 179.9);
    assert_eq!(info.name.as_deref(), Some("omo-sample-raster"));
    assert_eq!(
        info.attribution.as_deref(),
        Some("OMO test fixture (synthetic, public domain)")
    );
    let tile = layer.tile_rgba(2, 1, 3).unwrap().expect("tile exists");
    assert_eq!(tile.len(), 256 * 256 * 4);
    for (u, v) in [
        (0, 0),
        (1, 100),
        (2, 2),
        (10, 10),
        (128, 200),
        (253, 253),
        (255, 255),
    ] {
        assert_eq!(px(&tile, u, v), expected_pixel(2, 1, 3, u, v), "({u},{v})");
    }
}

#[test]
fn probe_computes_a_fingerprint_only_on_request() {
    let layer = PmtilesLayer::open(&fixture("sample-raster.pmtiles")).unwrap();
    assert!(layer.info(false).fingerprint_sha256.is_none());
    let info = layer.info(true);
    let bytes = std::fs::read(fixture("sample-raster.pmtiles")).unwrap();
    assert_eq!(info.file_size, bytes.len() as u64);
    assert_eq!(
        info.fingerprint_sha256.unwrap(),
        hex::encode(Sha256::digest(&bytes))
    );
}

#[test]
fn webp_lossless_matches_png_exactly_and_jpeg_is_close() {
    let png = PmtilesLayer::open(&fixture("sample-raster.pmtiles")).unwrap();
    let webp = PmtilesLayer::open(&fixture("sample-raster-webp.pmtiles")).unwrap();
    assert_eq!(webp.info(false).tile_kind, PmtilesTileKind::RasterWebp);
    let a = png.tile_rgba(2, 3, 1).unwrap().unwrap();
    let b = webp.tile_rgba(2, 3, 1).unwrap().unwrap();
    assert_eq!(a, b, "lossless WebP must equal the PNG pixels");

    let jpeg = PmtilesLayer::open(&fixture("sample-raster-jpeg.pmtiles")).unwrap();
    assert_eq!(jpeg.info(false).tile_kind, PmtilesTileKind::RasterJpeg);
    let c = jpeg.tile_rgba(2, 3, 1).unwrap().unwrap();
    assert_eq!(c.len(), 256 * 256 * 4);
    // JPEG has no alpha: opaque; color stays near the source away from edges.
    let want = expected_pixel(2, 3, 1, 128, 128);
    let got = px(&c, 128, 128);
    assert_eq!(got[3], 255);
    for ch in 0..3 {
        assert!(got[ch].abs_diff(want[ch]) <= 12, "{got:?} vs {want:?}");
    }
}

#[test]
fn raster_overzoom_uses_the_ancestor_tile_scaled_up() {
    let layer = PmtilesLayer::open(&fixture("sample-raster.pmtiles")).unwrap();
    // maxzoom is 3: z5 tile (13, 22) descends from z3 tile (3, 5) with dz = 2.
    let over = layer
        .tile_rgba(5, 13, 22)
        .unwrap()
        .expect("overzoomed tile");
    let ancestor = layer.tile_rgba(3, 3, 5).unwrap().unwrap();
    for (u, v) in [
        (0usize, 0usize),
        (17, 90),
        (128, 128),
        (255, 255),
        (200, 31),
    ] {
        let (sub_x, sub_y) = (13usize & 3, 22usize & 3);
        let su = (sub_x * 256 + u) >> 2;
        let sv = (sub_y * 256 + v) >> 2;
        assert_eq!(px(&over, u, v), px(&ancestor, su, sv), "({u},{v})");
    }
    // The deepest zoom still resolves.
    assert!(layer.tile_rgba(12, 2048, 1500).unwrap().is_some());
}

#[test]
fn mvt_gzip_and_zstd_archives_rasterize_identically() {
    let gz = PmtilesLayer::open(&fixture("sample-mvt.pmtiles")).unwrap();
    let zstd = PmtilesLayer::open(&fixture("sample-mvt-zstd.pmtiles")).unwrap();
    assert_eq!(gz.info(false).tile_kind, PmtilesTileKind::VectorMvt);
    assert_eq!(zstd.info(false).tile_kind, PmtilesTileKind::VectorMvt);
    let a = gz.tile_rgba(1, 0, 0).unwrap().unwrap();
    let b = zstd.tile_rgba(1, 0, 0).unwrap().unwrap();
    assert_eq!(a.len(), 256 * 256 * 4);
    assert_eq!(a, b, "tile compression must not change the picture");
    // Deterministic across repeated renders.
    assert_eq!(a, gz.tile_rgba(1, 0, 0).unwrap().unwrap());
    // Water (left half) and earth (right half) get different, opaque fills.
    let water = px(&a, 20, 200);
    let earth = px(&a, 240, 240);
    assert_eq!(water[3], 255);
    assert_eq!(earth[3], 255);
    assert_ne!(water, earth);
    // The boundary polyline leaves a darker, distinct stroke somewhere along its path.
    let distinct: std::collections::HashSet<[u8; 4]> = (0..256)
        .flat_map(|v| (0..256).map(move |u| (u, v)))
        .map(|(u, v)| px(&a, u, v))
        .collect();
    assert!(
        distinct.len() > 3,
        "expected fills plus antialiased boundary pixels"
    );
}

#[test]
fn mvt_overzoom_renders_the_ancestor_geometry() {
    let layer = PmtilesLayer::open(&fixture("sample-mvt.pmtiles")).unwrap();
    let tile = layer
        .tile_rgba(6, 5, 7)
        .unwrap()
        .expect("overzoomed vector tile");
    assert_eq!(tile.len(), 256 * 256 * 4);
    // Fully inside the earth/water fill, so every pixel is opaque.
    let (pixels, rem) = tile.as_chunks::<4>();
    assert!(rem.is_empty() && pixels.iter().all(|p| p[3] == 255));
}

#[test]
fn missing_tiles_and_out_of_range_zooms_are_transparent_not_errors() {
    let layer = PmtilesLayer::open(&fixture("sample-raster.pmtiles")).unwrap();
    // z0 is the archive minzoom; the app never asks below z1, but z0 resolves fine.
    assert!(layer.tile_rgba(1, 0, 0).unwrap().is_some());
}

// ---- synthetic archives ----------------------------------------------------------------

fn varint(mut n: u64, out: &mut Vec<u8>) {
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

/// `(z, x, y, payload)` tiles -> single-root-directory PMTiles v3 (gzip directories,
/// uncompressed tiles of `tile_type`).
fn synthetic(
    tile_type: u8,
    min_zoom: u8,
    max_zoom: u8,
    bounds_deg: [f64; 4],
    tiles: &[(u8, u32, u32, Vec<u8>)],
    version: u8,
    magic: &[u8; 7],
) -> Vec<u8> {
    let mut sorted: Vec<_> = tiles
        .iter()
        .map(|(z, x, y, p)| (tile_id(*z, *x, *y), p))
        .collect();
    sorted.sort_by_key(|t| t.0);
    let mut data = Vec::new();
    let mut entries = Vec::new();
    for (id, payload) in &sorted {
        entries.push((*id, data.len() as u64, payload.len() as u64));
        data.extend_from_slice(payload);
    }
    let mut dir = Vec::new();
    varint(entries.len() as u64, &mut dir);
    let mut last = 0;
    for (id, _, _) in &entries {
        varint(id - last, &mut dir);
        last = *id;
    }
    for _ in &entries {
        varint(1, &mut dir);
    }
    for (_, _, len) in &entries {
        varint(*len, &mut dir);
    }
    for (_, off, _) in &entries {
        varint(off + 1, &mut dir);
    }
    let root = gzip(&dir);
    let meta = gzip(br#"{"name":"synthetic","attribution":"(c) synthetic"}"#);
    let root_off = 127u64;
    let meta_off = root_off + root.len() as u64;
    let data_off = meta_off + meta.len() as u64;
    let e7 = |v: f64| (v * 1e7).round() as i32;
    let mut h = Vec::new();
    h.extend_from_slice(magic);
    h.push(version);
    for v in [
        root_off,
        root.len() as u64,
        meta_off,
        meta.len() as u64,
        data_off,
        0,
    ] {
        h.extend_from_slice(&v.to_le_bytes());
    }
    for v in [
        data_off,
        data.len() as u64,
        tiles.len() as u64,
        tiles.len() as u64,
        tiles.len() as u64,
    ] {
        h.extend_from_slice(&v.to_le_bytes());
    }
    h.extend_from_slice(&[1, 2, 1, tile_type, min_zoom, max_zoom]);
    for v in bounds_deg {
        h.extend_from_slice(&e7(v).to_le_bytes());
    }
    h.push(min_zoom);
    h.extend_from_slice(&0i32.to_le_bytes());
    h.extend_from_slice(&0i32.to_le_bytes());
    assert_eq!(h.len(), 127);
    [h, root, meta, data].concat()
}

fn red_png() -> Vec<u8> {
    let mut img = image::RgbaImage::new(256, 256);
    for p in img.pixels_mut() {
        *p = image::Rgba([200, 20, 20, 255]);
    }
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn write_tmp(dir: &tempfile::TempDir, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn minzoom_and_bounds_clamp_to_transparent() {
    let dir = tempfile::tempdir().unwrap();
    // Bounds: east Asia only. minzoom 3, maxzoom 4. Tiles exist for (3,6,3) and (4,13,6).
    let bytes = synthetic(
        2,
        3,
        4,
        [90.0, 10.0, 140.0, 55.0],
        &[(3, 6, 3, red_png()), (4, 13, 6, red_png())],
        3,
        b"PMTiles",
    );
    let layer = PmtilesLayer::open(&write_tmp(&dir, "bounded.pmtiles", &bytes)).unwrap();
    let info = layer.info(false);
    assert_eq!((info.min_zoom, info.max_zoom), (3, 4));
    assert!((info.bounds[0] - 90.0).abs() < 1e-6 && (info.bounds[3] - 55.0).abs() < 1e-6);
    // Below minzoom: transparent even though (3,6,3) covers z2's ancestor area.
    assert!(layer.tile_rgba(2, 3, 1).unwrap().is_none());
    // Inside bounds + present.
    let hit = layer.tile_rgba(3, 6, 3).unwrap().unwrap();
    assert_eq!(px(&hit, 100, 100), [200, 20, 20, 255]);
    // Outside the declared bounds (Americas): transparent.
    assert!(layer.tile_rgba(3, 1, 3).unwrap().is_none());
    // Inside bounds but absent from the archive: transparent.
    assert!(layer.tile_rgba(4, 12, 6).unwrap().is_none());
    // Overzoom above maxzoom still resolves from the maxzoom ancestor.
    let over = layer.tile_rgba(6, 52, 24).unwrap().unwrap();
    assert_eq!(px(&over, 5, 5), [200, 20, 20, 255]);
}

#[test]
fn five_hundred_twelve_pixel_raster_tiles_are_rescaled_to_256() {
    let dir = tempfile::tempdir().unwrap();
    let mut img = image::RgbaImage::new(512, 512);
    for p in img.pixels_mut() {
        *p = image::Rgba([10, 120, 200, 255]);
    }
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let bytes = synthetic(
        2,
        0,
        1,
        [-180.0, -85.0, 180.0, 85.0],
        &[(1, 0, 0, png.into_inner())],
        3,
        b"PMTiles",
    );
    let layer = PmtilesLayer::open(&write_tmp(&dir, "big.pmtiles", &bytes)).unwrap();
    let tile = layer.tile_rgba(1, 0, 0).unwrap().unwrap();
    assert_eq!(tile.len(), 256 * 256 * 4);
    assert_eq!(px(&tile, 77, 190), [10, 120, 200, 255]);
}

#[test]
fn corrupt_archives_report_structured_errors_without_panicking() {
    // Valid header and directory, undecodable tile payloads: error is per tile.
    let layer = PmtilesLayer::open(&fixture("corrupt.pmtiles")).unwrap();
    assert!(matches!(layer.smoke_check(), Err(PmtilesError::Corrupt(_))));
    for good in [
        "sample-raster",
        "sample-mvt",
        "sample-mvt-zstd",
        "sample-raster-webp",
    ] {
        PmtilesLayer::open(&fixture(&format!("{good}.pmtiles")))
            .unwrap()
            .smoke_check()
            .unwrap_or_else(|e| panic!("{good}: {e}"));
    }
    let err = layer.tile_rgba(1, 0, 0).unwrap_err();
    assert!(matches!(err, PmtilesError::Corrupt(_)), "{err:?}");
    assert!(err.to_string().contains("tile"), "{err}");

    let dir = tempfile::tempdir().unwrap();
    let missing = PmtilesLayer::open(&dir.path().join("nope.pmtiles")).unwrap_err();
    assert!(matches!(missing, PmtilesError::Io(_)), "{missing:?}");

    let garbage = write_tmp(&dir, "garbage.pmtiles", &[7u8; 400]);
    assert!(matches!(
        PmtilesLayer::open(&garbage).unwrap_err(),
        PmtilesError::NotPmtiles
    ));
    let tiny = write_tmp(&dir, "tiny.pmtiles", b"PMTiles");
    assert!(PmtilesLayer::open(&tiny).is_err());

    let v2 = synthetic(
        2,
        0,
        0,
        [-180.0, -85.0, 180.0, 85.0],
        &[(0, 0, 0, red_png())],
        2,
        b"PMTiles",
    );
    assert!(matches!(
        PmtilesLayer::open(&write_tmp(&dir, "v2.pmtiles", &v2)).unwrap_err(),
        PmtilesError::UnsupportedVersion(2)
    ));

    // Truncated archive: the directory points beyond the end of the file.
    let ok = synthetic(
        2,
        0,
        0,
        [-180.0, -85.0, 180.0, 85.0],
        &[(0, 0, 0, red_png())],
        3,
        b"PMTiles",
    );
    let cut = write_tmp(&dir, "cut.pmtiles", &ok[..ok.len() - 100]);
    match PmtilesLayer::open(&cut) {
        Err(_) => {}
        Ok(layer) => assert!(layer.tile_rgba(0, 0, 0).is_err()),
    }
}

#[test]
fn unsupported_tile_types_are_rejected_at_open() {
    let dir = tempfile::tempdir().unwrap();
    // tile type 5 = AVIF
    let bytes = synthetic(
        5,
        0,
        0,
        [-180.0, -85.0, 180.0, 85.0],
        &[(0, 0, 0, vec![1, 2, 3])],
        3,
        b"PMTiles",
    );
    let err = PmtilesLayer::open(&write_tmp(&dir, "avif.pmtiles", &bytes)).unwrap_err();
    assert!(matches!(err, PmtilesError::Unsupported(_)), "{err:?}");
}

#[test]
fn zstd_internal_directory_compression_is_rejected_with_structured_diagnostic() {
    let dir = tempfile::tempdir().unwrap();
    let mut bytes = synthetic(
        2,
        0,
        0,
        [-180.0, -85.0, 180.0, 85.0],
        &[(0, 0, 0, red_png())],
        3,
        b"PMTiles",
    );
    // Byte 97 is internal_compression (4 = Zstd).
    bytes[97] = 4;
    let err = PmtilesLayer::open(&write_tmp(&dir, "zstd-dir.pmtiles", &bytes)).unwrap_err();
    assert!(
        matches!(&err, PmtilesError::Unsupported(msg) if msg.contains("Zstd")),
        "{err:?}"
    );
}
