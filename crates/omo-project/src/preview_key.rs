use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::map::{MapLayerConfig, opacity_bits};

/// Closed lead-time window in hours. Equality participates in preview identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PreviewWindow {
    pub start_h: u32,
    pub end_h: u32,
}

impl Default for PreviewWindow {
    fn default() -> Self {
        Self {
            start_h: 0,
            end_h: 168,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PreviewKey {
    pub doc_id: String,
    pub revision: u64,
    pub view_id: String,
    pub query_hash: String,
    pub stations_hash: Option<String>,
    pub catalog_version: String,
    pub run_ref: String,
    pub window: PreviewWindow,
    pub max_distance_m: u32,
    pub worker_generation: u64,
}

impl PreviewKey {
    pub fn m1_defaults() -> Self {
        Self {
            doc_id: String::new(),
            revision: 0,
            view_id: String::new(),
            query_hash: String::new(),
            stations_hash: None,
            catalog_version: String::new(),
            run_ref: "latest_complete".into(),
            window: PreviewWindow::default(),
            max_distance_m: 7200,
            worker_generation: 0,
        }
    }
}

/// Identity of a displayed map (`FR-020`). A mismatch against current project/worker state
/// marks the map stale while the last good tiles stay on screen.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct MapPreviewKey {
    pub doc_id: String,
    pub revision: u64,
    pub stations_hash: Option<String>,
    pub catalog_entry_id: String,
    pub catalog_version: String,
    pub run_ref: String,
    pub valid_time_utc: String,
    pub color_palette: String,
    /// `f64::to_bits` of `(min, max)` after `-0.0 -> 0.0` normalisation.
    pub color_locked_range: Option<(u64, u64)>,
    /// Canonical digest of visible layers, opacities, and order.
    pub layers_hash: String,
    pub worker_generation: u64,
}

impl MapPreviewKey {
    /// A locked range exists only when both bounds are finite and `min < max`.
    pub fn normalized_range(min: Option<f64>, max: Option<f64>) -> Option<(u64, u64)> {
        let (min, max) = (min?, max?);
        if !min.is_finite() || !max.is_finite() || min >= max {
            return None;
        }
        let fix = |v: f64| {
            if v == 0.0 {
                0.0f64.to_bits()
            } else {
                v.to_bits()
            }
        };
        Some((fix(min), fix(max)))
    }

    /// SHA-256 over the ordered visible layers (`id`, kind, opacity bits).
    pub fn layers_digest(layers: &[MapLayerConfig]) -> String {
        let mut hasher = Sha256::new();
        for layer in layers.iter().filter(|layer| layer.visible) {
            let kind = serde_json::to_string(&layer.kind).unwrap_or_default();
            hasher.update(format!(
                "{}\u{1f}{}\u{1f}{}\n",
                layer.id,
                kind,
                opacity_bits(layer.opacity)
            ));
        }
        format!("{:x}", hasher.finalize())
    }

    pub fn is_stale_against(&self, current: &MapPreviewKey) -> bool {
        self != current
    }
}
