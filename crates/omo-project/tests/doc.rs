use omo_project::{doc::ProjectDoc, preview_key::PreviewKey};

fn key(revision: u64) -> PreviewKey {
    PreviewKey {
        doc_id: "doc-1".into(),
        revision,
        view_id: "sql-1".into(),
        query_hash: "query-hash".into(),
        stations_hash: None,
        catalog_version: "catalog-0".into(),
        worker_generation: 3,
    }
}

#[test]
fn revision_and_dirty_follow_edits_and_saves() {
    let mut doc = ProjectDoc::new("doc-1");
    assert_eq!(doc.revision(), 0);
    assert!(!doc.is_dirty());
    doc.record_edit().unwrap();
    assert_eq!(doc.revision(), 1);
    assert!(doc.is_dirty());
    doc.mark_saved();
    assert!(!doc.is_dirty());
    doc.record_edit().unwrap();
    assert!(doc.is_dirty());
}

#[test]
fn preview_key_equality_covers_every_input() {
    assert_eq!(key(2), key(2));
    assert_ne!(key(2), key(3));
    let mut changed = key(2);
    changed.worker_generation += 1;
    assert_ne!(key(2), changed);
}
