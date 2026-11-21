//! Plot sampling limits shared by the worker and UI.

/// Maximum numeric columns retained in a plot preview.
pub const MAX_PLOT_SERIES: usize = 20;
/// Maximum rows retained in a plot preview per numeric series.
pub const MAX_PLOT_POINTS: usize = 10_000;
