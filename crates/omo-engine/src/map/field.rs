//! Worker-resident forecast fields (`data-model.md` §4): dense O1280 values plus the
//! revision-wide statistics that keep colors uniform across tiles (`FR-010`).

use omo_protocol::map::{FieldStats, MapBbox, SourceLayout};

use super::raster::{FieldSampler, O1280Sampler, RegularGrid, RegularSampler};
use crate::grid::reduced_gaussian::O1280;

/// Scalar values for a cached forecast field (`O1280` reduced Gaussian or `RegularGrid` lon/lat);
/// `NAN` marks cells that were not loaded or that the source reports as missing.
#[derive(Clone, Debug)]
pub enum GridValues {
    O1280Dense(Vec<f32>),
    Regular(RegularGrid),
}

impl GridValues {
    pub fn o1280_empty() -> Self {
        Self::O1280Dense(vec![f32::NAN; O1280::points() as usize])
    }

    pub fn as_slice(&self) -> &[f32] {
        match self {
            Self::O1280Dense(values) => values,
            Self::Regular(grid) => &grid.values,
        }
    }

    pub fn as_mut_slice(&mut self) -> &mut [f32] {
        match self {
            Self::O1280Dense(values) => values,
            Self::Regular(grid) => &mut grid.values,
        }
    }

    pub fn get(&self, flat: u32) -> f32 {
        self.as_slice()[flat as usize]
    }

    pub fn sampler(&self) -> FieldSampler<'_> {
        match self {
            Self::O1280Dense(values) => FieldSampler::O1280(O1280Sampler::new(values)),
            Self::Regular(grid) => FieldSampler::Regular(RegularSampler::new(grid)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct CachedField {
    pub field_id: String,
    pub entry_id: String,
    pub reference_time_utc: String,
    pub valid_time_utc: String,
    pub variable: String,
    pub unit: String,
    pub source_layout: SourceLayout,
    pub source_object: String,
    pub object_version: Option<String>,
    /// Region that was actually requested; cells outside it stay `NAN`.
    pub coverage: MapBbox,
    pub values: GridValues,
    pub stats: FieldStats,
}

impl CachedField {
    /// Resident size in bytes (for memory accounting and tests).
    pub fn resident_bytes(&self) -> usize {
        std::mem::size_of_val(self.values.as_slice())
    }
}

/// Minimum, maximum and finite-cell count over a value slice.
pub fn compute_stats(values: &[f32], unit: &str) -> FieldStats {
    let mut valid_count = 0u64;
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for &value in values {
        if value.is_finite() {
            valid_count += 1;
            min = min.min(value);
            max = max.max(value);
        }
    }
    if valid_count == 0 {
        return FieldStats {
            valid_count: 0,
            min: 0.0,
            max: 0.0,
            unit: unit.to_owned(),
        };
    }
    FieldStats {
        valid_count,
        min: f64::from(min),
        max: f64::from(max),
        unit: unit.to_owned(),
    }
}

/// Bounded LRU of loaded fields: at most [`FieldCache::CAPACITY`] (`<= 53 MB` resident).
#[derive(Debug, Default)]
pub struct FieldCache {
    /// Least recently used first.
    entries: Vec<CachedField>,
}

impl FieldCache {
    pub const CAPACITY: usize = 2;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Insert a field (replacing any field with the same id), evicting the least recently used
    /// entry when over capacity. Returns the evicted `field_id`s.
    pub fn insert(&mut self, field: CachedField) -> Vec<String> {
        self.entries
            .retain(|entry| entry.field_id != field.field_id);
        self.entries.push(field);
        let mut evicted = Vec::new();
        while self.entries.len() > Self::CAPACITY {
            evicted.push(self.entries.remove(0).field_id);
        }
        evicted
    }

    /// Look up a field and mark it most recently used.
    pub fn get(&mut self, field_id: &str) -> Option<&CachedField> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.field_id == field_id)?;
        let entry = self.entries.remove(index);
        self.entries.push(entry);
        self.entries.last()
    }

    /// Look up without changing recency.
    pub fn peek(&self, field_id: &str) -> Option<&CachedField> {
        self.entries.iter().find(|entry| entry.field_id == field_id)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}
