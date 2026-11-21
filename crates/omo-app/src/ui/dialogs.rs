//! Native file picker adapters and the manual-path export fallback.

use std::path::PathBuf;

use nsh_base::alert_dialog;
use nsh_core::{
    file_dialog::{FileFilter, OpenDialog, SaveDialog},
    prelude::*,
};

pub(crate) fn choose_stations_csv(
    on_selected: impl Fn(PathBuf) + 'static,
    on_error: impl Fn(String) + 'static,
) {
    spawn(async move {
        #[cfg(feature = "dialog-test-seams")]
        if std::env::var_os("OMOBENCH_TEST_PORTAL_ERROR").is_some() {
            on_error("simulated portal failure: no file chooser service".into());
            return;
        }

        let answer = pick_paths(
            OpenDialog::files()
                .filter(FileFilter::new("CSV files", ["csv"]))
                .title("Open station data")
                .button_label("Open"),
        )
        .await;

        match answer {
            Ok(Some(paths)) => {
                if let Some(path) = paths.into_iter().next() {
                    on_selected(path);
                }
            }
            Ok(None) => {}
            Err(error) => on_error(error),
        }
    });
}

pub(crate) fn choose_export_path(
    on_selected: impl Fn(PathBuf, bool) + 'static,
    on_error: impl Fn(String) + 'static,
) {
    spawn(async move {
        let answer = pick_save_path(
            SaveDialog::new()
                .file_name("omobench-results.csv")
                .filter(FileFilter::new("CSV or Parquet files", ["csv", "parquet"]))
                .title("Export results")
                .button_label("Choose destination"),
        )
        .await;

        match answer {
            Ok(Some(path)) => confirm_export_path(path, on_selected, on_error),
            Ok(None) => {}
            Err(error) => on_error(error),
        }
    });
}

pub(crate) fn confirm_export_path(
    path: PathBuf,
    on_selected: impl Fn(PathBuf, bool) + 'static,
    on_error: impl Fn(String) + 'static,
) {
    spawn(async move {
        let sidecar = omo_protocol::export::sidecar_path(&path);
        let path_for_check = path.clone();
        match blocking(move || path_for_check.exists() || sidecar.exists()).await {
            Ok(true) => {
                let confirmation = alert_dialog()
                    .title("Replace this export?")
                    .description("The destination or its provenance sidecar already exists.")
                    .cancel("cancel", "Cancel")
                    .destructive("overwrite", "Replace")
                    .show()
                    .await;
                match confirmation {
                    Ok(answer) if answer.action.as_deref() == Some("overwrite") => {
                        on_selected(path, true);
                    }
                    Ok(_) => {}
                    Err(error) => on_error(error),
                }
            }
            Ok(false) => on_selected(path, false),
            Err(error) => on_error(format!("cannot inspect export destination: {error}")),
        }
    });
}
