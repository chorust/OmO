//! Minimal vector text for the composite map export: DejaVu Sans outlines filled with
//! `tiny-skia`. Pure Rust and byte-for-byte deterministic across platforms (no system fonts, no
//! shaping, no kerning), which is all a header, a colorbar label and an attribution footer need.
//!
//! The font is `assets/DejaVuSans.ttf`, copied unmodified from Debian's `fonts-dejavu-core`;
//! its licence is `assets/DejaVuSans-LICENSE.txt`.

use std::sync::OnceLock;

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Transform};
use ttf_parser::{Face, GlyphId, OutlineBuilder};

static FONT_BYTES: &[u8] = include_bytes!("../../assets/DejaVuSans.ttf");

fn face() -> &'static Face<'static> {
    static FACE: OnceLock<Face<'static>> = OnceLock::new();
    FACE.get_or_init(|| Face::parse(FONT_BYTES, 0).expect("the embedded font is a valid TTF"))
}

fn glyph(ch: char) -> GlyphId {
    face().glyph_index(ch).unwrap_or(GlyphId(0))
}

fn scale(size: f32) -> f32 {
    size / f32::from(face().units_per_em())
}

fn advance(ch: char, size: f32) -> f32 {
    face()
        .glyph_hor_advance(glyph(ch))
        .map_or(0.0, |units| f32::from(units) * scale(size))
}

/// Width in pixels of `text` set at `size` pixels per em.
pub fn width(text: &str, size: f32) -> f32 {
    text.chars().map(|ch| advance(ch, size)).sum()
}

struct Outline<'a> {
    path: &'a mut PathBuilder,
    scale: f32,
    x: f32,
    y: f32,
}

impl Outline<'_> {
    fn point(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.scale, self.y - y * self.scale)
    }
}

impl OutlineBuilder for Outline<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.path.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.path.line_to(x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.point(x1, y1);
        let (x, y) = self.point(x, y);
        self.path.quad_to(x1, y1, x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.point(x1, y1);
        let (x2, y2) = self.point(x2, y2);
        let (x, y) = self.point(x, y);
        self.path.cubic_to(x1, y1, x2, y2, x, y);
    }
    fn close(&mut self) {
        self.path.close();
    }
}

/// Draw `text` with its baseline at `y` starting at `x`.
pub fn draw(pixmap: &mut Pixmap, text: &str, x: f32, y: f32, size: f32, color: [u8; 3]) {
    let mut path = PathBuilder::new();
    let mut pen = x;
    let factor = scale(size);
    for ch in text.chars() {
        let id = glyph(ch);
        face().outline_glyph(
            id,
            &mut Outline {
                path: &mut path,
                scale: factor,
                x: pen,
                y,
            },
        );
        pen += advance(ch, size);
    }
    let Some(path) = path.finish() else { return };
    let mut paint = Paint::default();
    paint.set_color_rgba8(color[0], color[1], color[2], 255);
    paint.anti_alias = true;
    pixmap.fill_path(
        &path,
        &paint,
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

/// `text` shortened with an ellipsis until it fits `max_width`.
pub fn fit(text: &str, size: f32, max_width: f32) -> String {
    if width(text, size) <= max_width {
        return text.to_owned();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>() + "…";
        if width(&candidate, size) <= max_width {
            return candidate;
        }
    }
    String::new()
}

/// Break `text` into lines no wider than `max_width`, at spaces where possible.
pub fn wrap(text: &str, size: f32, max_width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if width(&candidate, size) <= max_width {
            line = candidate;
            continue;
        }
        if !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        // A word wider than a line is broken by characters.
        let mut rest = word.to_owned();
        while width(&rest, size) > max_width {
            let mut cut = rest.chars().count();
            while cut > 1 && width(&rest.chars().take(cut).collect::<String>(), size) > max_width {
                cut -= 1;
            }
            lines.push(rest.chars().take(cut).collect());
            rest = rest.chars().skip(cut).collect();
        }
        line = rest;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_scale_with_size_and_cover_the_symbols_the_export_uses() {
        let w10 = width("Natural Earth © °C …", 10.0);
        assert!((width("Natural Earth © °C …", 20.0) - 2.0 * w10).abs() < 0.01);
        for ch in ['©', '°', '…', '±', '·'] {
            assert!(face().glyph_index(ch).is_some(), "{ch}");
        }
    }

    #[test]
    fn wrap_respects_the_width_and_loses_nothing() {
        let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
        let lines = wrap(text, 11.0, 90.0);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| width(l, 11.0) <= 90.0));
        assert_eq!(lines.join(" "), text);
        let long = wrap(&"x".repeat(200), 11.0, 90.0);
        assert!(long.iter().all(|l| width(l, 11.0) <= 90.0));
        assert_eq!(long.concat(), "x".repeat(200));
    }

    #[test]
    fn fit_adds_an_ellipsis_only_when_needed() {
        assert_eq!(fit("short", 12.0, 500.0), "short");
        let fitted = fit("a rather long title that cannot fit", 12.0, 80.0);
        assert!(fitted.ends_with('…') && width(&fitted, 12.0) <= 80.0);
    }
}
