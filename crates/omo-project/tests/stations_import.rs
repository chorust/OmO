//! Station CSV import contract (contracts/station-csv-import.md, research R-04).

use omo_project::stations::import::{confirm, import_csv, sniff_delimiter, write_canonical};
use omo_project::stations::{ColumnMapping, ImportError, RowIssueKind};

#[test]
fn comma_and_semicolon_are_sniffed_and_both_are_ambiguous() {
    assert_eq!(sniff_delimiter("id,lat,lon").unwrap(), b',');
    assert_eq!(sniff_delimiter("id;lat;lon").unwrap(), b';');
    assert!(matches!(
        sniff_delimiter("id,lat;lon"),
        Err(ImportError::AmbiguousDelimiter)
    ));
}

#[test]
fn bom_quotes_aliases_normalization_and_all_row_issues() {
    let bytes = "\u{feff}id;latitude;longitude;name\nA1;39.9;116.4;\"北京\"\nA1;10;20;dup\n;91;10;bad\nx;nope;10;nan\ny;10;400;range\nz;1;190;wrap\n".as_bytes();
    let parsed = import_csv(bytes, "stations.csv", "2026-11-23T00:00:00Z").unwrap();
    assert_eq!(parsed.delimiter, b';');
    assert_eq!(parsed.proposed_mapping.id, "id");
    assert_eq!(parsed.proposed_mapping.lat, "latitude");
    assert_eq!(parsed.sample_rows[0][3], "北京");
    let error = confirm(
        &parsed,
        parsed.proposed_mapping.clone(),
        "stations.csv",
        bytes,
        "2026-11-23T00:00:00Z",
    )
    .unwrap_err();
    let ImportError::Rows(issues) = error else {
        panic!("expected row issues, got {error:?}");
    };
    assert!(
        issues
            .iter()
            .any(|issue| matches!(issue.kind, RowIssueKind::Duplicate { first_line: 2 }))
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.kind == RowIssueKind::Empty && issue.line == 4)
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.kind == RowIssueKind::OutOfRange && issue.field == "latitude")
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.kind == RowIssueKind::NotNumber)
    );
    assert!(
        issues
            .iter()
            .any(|issue| issue.kind == RowIssueKind::OutOfRange && issue.field == "longitude")
    );

    let good = "code,y,x,station_name\nS1,31.2,121.5,\"上海\"\nS2,0,180,dateline\n".as_bytes();
    let parsed = import_csv(good, "/tmp/s.csv", "2026-11-23T00:00:00Z").unwrap();
    assert_eq!(parsed.proposed_mapping.id, "code");
    assert_eq!(parsed.proposed_mapping.lat, "y");
    assert_eq!(parsed.proposed_mapping.lon, "x");
    assert_eq!(
        parsed.proposed_mapping.name.as_deref(),
        Some("station_name")
    );
    let mapping = parsed.proposed_mapping.clone();
    let snapshot = confirm(
        &parsed,
        mapping.clone(),
        "/tmp/s.csv",
        good,
        "2026-11-23T00:00:00Z",
    )
    .unwrap();
    assert_eq!(snapshot.rows[1].longitude, -180.0);
    assert_eq!(snapshot.rows[0].name.as_deref(), Some("上海"));
    let again = confirm(&parsed, mapping, "/tmp/s.csv", good, "2026-11-23T00:00:00Z").unwrap();
    assert_eq!(snapshot.content_sha256, again.content_sha256);
    assert_eq!(snapshot.content_sha256.len(), 64);
}

#[test]
fn encoding_empty_and_incomplete_mapping_fail_closed() {
    let error = import_csv(&[0xff, 0xfe, 0x00], "x.csv", "2026-11-23T00:00:00Z").unwrap_err();
    assert!(matches!(error, ImportError::Encoding { byte_offset: 0 }));
    assert!(matches!(
        import_csv(b"", "x.csv", "2026-11-23T00:00:00Z").unwrap_err(),
        ImportError::Empty
    ));
    let parsed = import_csv(b"name,elevation\nA,10\n", "x.csv", "2026-11-23T00:00:00Z").unwrap();
    let error = confirm(
        &parsed,
        ColumnMapping {
            id: String::new(),
            lat: String::new(),
            lon: String::new(),
            name: None,
            elevation: None,
        },
        "x.csv",
        b"name,elevation\nA,10\n",
        "2026-11-23T00:00:00Z",
    )
    .unwrap_err();
    assert!(matches!(error, ImportError::MappingIncomplete));
}

#[test]
fn canonical_sha256_of_m1_100_is_stable() {
    let bytes = include_bytes!("../../../fixtures/stations/m1-100.csv");
    let parsed = import_csv(
        bytes,
        "fixtures/stations/m1-100.csv",
        "2026-11-23T00:00:00Z",
    )
    .unwrap();
    let snapshot = confirm(
        &parsed,
        parsed.proposed_mapping.clone(),
        "fixtures/stations/m1-100.csv",
        bytes,
        "2026-11-23T00:00:00Z",
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = write_canonical(&snapshot, dir.path()).unwrap();
    assert!(path.ends_with(format!("stations-{}.csv", snapshot.content_sha256)));
    assert_eq!(
        snapshot.content_sha256,
        "8dfb18f5c208e6f32882bdfd841f72019af98749cae0e9620db49dbf245e0dfd"
    );
}
