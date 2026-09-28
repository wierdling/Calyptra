//! Palette file import: Fractint `.map`, GIMP `.ggr`, Ultra Fractal `.ugr`.

use std::path::Path;

use crate::{Gradient, Stop};

#[derive(Debug)]
pub enum ImportError {
    Io(std::io::Error),
    UnknownFormat(String),
    Parse(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::UnknownFormat(ext) => {
                write!(
                    f,
                    "unsupported palette format {ext:?} (use .map, .ggr or .ugr)"
                )
            }
            Self::Parse(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ImportError {}

/// OKLab error below which imported stops are merged (roughly the smallest
/// visible difference).
const SIMPLIFY_TOLERANCE: f32 = 0.008;

/// Loads a palette file, choosing the parser by extension, and simplifies it
/// to the stops that matter.
pub fn import_palette(path: &Path) -> Result<Gradient, ImportError> {
    let text = std::fs::read_to_string(path).map_err(ImportError::Io)?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut gradient = match ext.as_str() {
        "map" => parse_map(&text),
        "ggr" => parse_ggr(&text),
        "ugr" => parse_ugr(&text),
        _ => return Err(ImportError::UnknownFormat(ext)),
    }
    .map_err(ImportError::Parse)?;
    gradient.simplify(SIMPLIFY_TOLERANCE);
    Ok(gradient)
}

/// Fractint `.map`: one `r g b` (0–255) triple per line, anything after the
/// third number is a comment.
pub fn parse_map(text: &str) -> Result<Gradient, String> {
    let mut colors = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let fields: Vec<&str> = line.split_whitespace().take(3).collect();
        if fields.is_empty() {
            continue;
        }
        let parsed: Result<Vec<u8>, _> = fields.iter().map(|f| f.parse::<u8>()).collect();
        match parsed.as_deref() {
            Ok(&[r, g, b]) => colors.push([r, g, b]),
            _ => return Err(format!("line {}: expected `r g b` in 0-255", number + 1)),
        }
    }
    evenly_spaced(colors)
}

/// GIMP `.ggr`: segments `left middle right r0 g0 b0 a0 r1 g1 b1 a1 ...`
/// with colors in [0, 1]. The midpoint and blend function are approximated
/// by a straight OKLab blend.
pub fn parse_ggr(text: &str) -> Result<Gradient, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("GIMP Gradient") {
        return Err("not a GIMP gradient (missing header)".into());
    }
    let mut stops = Vec::new();
    for line in lines {
        let values: Vec<f32> = line
            .split_whitespace()
            .map_while(|f| f.parse().ok())
            .collect();
        // Skips `Name:` and the segment count (single number).
        if values.len() < 11 {
            continue;
        }
        let to_srgb8 = |rgb: &[f32]| {
            [rgb[0], rgb[1], rgb[2]].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
        };
        stops.push(Stop::new(values[0], to_srgb8(&values[3..6])));
        stops.push(Stop::new(values[2], to_srgb8(&values[7..10])));
    }
    if stops.is_empty() {
        return Err("no gradient segments found".into());
    }
    dedupe(&mut stops);
    Ok(Gradient { stops })
}

/// Ultra Fractal `.ugr`: the first gradient's `index=N color=C` pairs.
/// Indices run 0..400 and colors are Windows COLORREF (0x00BBGGRR).
pub fn parse_ugr(text: &str) -> Result<Gradient, String> {
    let body = text
        .split_once("gradient:")
        .map(|(_, rest)| rest)
        .ok_or("no `gradient:` section found")?;
    // Only the first gradient: stop at its closing brace.
    let body = body.split('}').next().unwrap_or(body);

    let mut stops = Vec::new();
    let mut index = None;
    for token in body.split_whitespace() {
        if let Some(value) = token.strip_prefix("index=") {
            index = value.parse::<f32>().ok();
        } else if let Some(value) = token.strip_prefix("color=")
            && let (Some(i), Ok(c)) = (index.take(), value.parse::<u32>())
        {
            let rgb = [c as u8, (c >> 8) as u8, (c >> 16) as u8];
            stops.push(Stop::new((i / 400.0).rem_euclid(1.0), rgb));
        }
    }
    if stops.is_empty() {
        return Err("no `index=… color=…` entries found".into());
    }
    stops.sort_by(|a, b| a.position.total_cmp(&b.position));
    // Ultra Fractal gradients wrap around: close the loop at both ends.
    let (first, last) = (stops[0], stops[stops.len() - 1]);
    let span = 1.0 - last.position + first.position;
    if span > 0.0 && first.position > 0.0 {
        let f = (1.0 - last.position) / span;
        let wrap = mix_srgb8(last.color, first.color, f);
        stops.insert(0, Stop::new(0.0, wrap));
        stops.push(Stop::new(1.0, wrap));
    }
    Ok(Gradient { stops })
}

fn mix_srgb8(a: [u8; 3], b: [u8; 3], f: f32) -> [u8; 3] {
    Gradient {
        stops: vec![Stop::new(0.0, a), Stop::new(1.0, b)],
    }
    .sample_srgb8(f)
}

fn evenly_spaced(colors: Vec<[u8; 3]>) -> Result<Gradient, String> {
    if colors.len() < 2 {
        return Err("a palette needs at least two colors".into());
    }
    let last = (colors.len() - 1) as f32;
    Ok(Gradient {
        stops: colors
            .into_iter()
            .enumerate()
            .map(|(i, c)| Stop::new(i as f32 / last, c))
            .collect(),
    })
}

/// Drops a stop identical to the previous one (shared segment boundaries).
fn dedupe(stops: &mut Vec<Stop>) {
    stops.dedup_by(|b, a| a.position == b.position && a.color == b.color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_files_parse_with_comments() {
        let g = parse_map("0 0 0  black\n255 0 0\n\n255 255 255 white\n").unwrap();
        assert_eq!(g.stops.len(), 3);
        assert_eq!(g.stops[1], Stop::new(0.5, [255, 0, 0]));
        assert!(parse_map("1 2\n").is_err());
    }

    #[test]
    fn ggr_segments_become_stops() {
        let text = "GIMP Gradient\nName: Test\n2\n\
            0.0 0.25 0.5 1 0 0 1 0 1 0 1 0 0\n\
            0.5 0.75 1.0 0 1 0 1 0 0 1 1 0 0\n";
        let g = parse_ggr(text).unwrap();
        // The shared green stop at 0.5 is merged.
        assert_eq!(g.stops.len(), 3);
        assert_eq!(g.stops[0].color, [255, 0, 0]);
        assert_eq!(g.stops[1], Stop::new(0.5, [0, 255, 0]));
        assert_eq!(g.stops[2].color, [0, 0, 255]);
    }

    #[test]
    fn ugr_uses_colorref_order_and_wraps() {
        let text = "demo {\ngradient:\n  title=\"demo\" smooth=yes\n  \
            index=100 color=255\n  index=300 color=16711680\n}\nother {\ngradient:\n index=0 color=0\n}\n";
        let g = parse_ugr(text).unwrap();
        let red = g.stops.iter().find(|s| s.position == 0.25).unwrap();
        let blue = g.stops.iter().find(|s| s.position == 0.75).unwrap();
        assert_eq!(red.color, [255, 0, 0]);
        assert_eq!(blue.color, [0, 0, 255]);
        // Wrapped endpoints match, so it tiles.
        assert_eq!(
            g.stops.first().unwrap().color,
            g.stops.last().unwrap().color
        );
        assert_eq!(g.stops.len(), 4);
    }
}
