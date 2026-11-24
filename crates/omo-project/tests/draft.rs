//! Draft staging, recovery listing, and discard tests (T053).

use std::fs;

use omo_project::{doc::ProjectDoc, draft};
use tempfile::tempdir;

#[test]
fn drafts_stage_under_app_data_recover_unsaved_or_newer_and_clear_on_save() {
    let dir = tempdir().unwrap();
    let app_data = dir.path().join("app_data");
    let proj_path = dir.path().join("project.omoproj");

    // Unsaved document draft is recoverable and never writes to project_path.
    let mut doc = ProjectDoc::new("draft-doc-1");
    doc.record_edit().unwrap();
    let staged = draft::stage(&doc, &app_data, "2026-11-23T08:00:00Z").unwrap();
    assert!(staged.starts_with(app_data.join("drafts")));
    assert!(!proj_path.exists());
    let list = draft::list_recoverable(&app_data).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].draft.doc_id, "draft-doc-1");

    // Discard deletes the draft.
    draft::discard(&app_data, "draft-doc-1").unwrap();
    assert!(!staged.exists());
    assert!(draft::list_recoverable(&app_data).unwrap().is_empty());

    // Saving clears any existing draft.
    doc.save_to_path(&proj_path, Some(&app_data)).unwrap();
    let staged2 = draft::stage(&doc, &app_data, "2026-11-23T08:05:00Z").unwrap();
    assert!(staged2.exists());
    doc.record_edit().unwrap();
    doc.save_to_path(&proj_path, Some(&app_data)).unwrap();
    assert!(!staged2.exists());
    assert!(fs::read(&proj_path).is_ok());
}
