//! Apophysis / JWildfire / flam3 `.flame` files (XML).
//!
//! Mapping notes:
//! - flam3 lists affine coefficients column-wise, `coefs="a d b e c f"` in
//!   our terms (x' = a·x + b·y + c, y' = d·x + e·y + f).
//! - flam3 draws +y downward, we draw it upward. A mirror cannot be pushed
//!   through nonlinear variations, so it is applied exactly where it is
//!   linear: the final transform's post-affine (adding a plain `linear`
//!   final transform if the flame has none).
//! - Color speed: flam3's `symmetry` s means speed (1 − s) / 2; newer files
//!   also write `color_speed` directly.
//! - Our 3D affine terms and 3D camera go in `calyptra_*` attributes, which
//!   other programs ignore, so our own files round-trip exactly. Files
//!   written before the rename use `fractals_*`, which is still read.
//! - Anything we cannot represent (unknown variations, more than 4
//!   variations per transform or 12 transforms) is reported as a warning.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;

use quick_xml::events::Event;
use scene::flame::{Affine, Flame, FlameCamera, Variation, VariationKind, Xform};

use crate::ExportError;

/// One flame from a file, with its palette and anything that could not be
/// represented exactly.
#[derive(Clone, Debug)]
pub struct ImportedFlame {
    pub name: String,
    pub flame: Flame,
    pub gradient: color::Gradient,
    pub warnings: Vec<String>,
}

/// Reads every flame in a `.flame` file.
pub fn import_flames(path: &Path) -> Result<Vec<ImportedFlame>, ExportError> {
    parse_flames(&std::fs::read_to_string(path)?)
}

/// Writes one flame (with `gradient` as its 256-color palette).
pub fn export_flame(
    path: &Path,
    name: &str,
    flame: &Flame,
    gradient: &color::Gradient,
) -> Result<(), ExportError> {
    std::fs::write(path, flame_to_xml(name, flame, gradient))?;
    Ok(())
}

// ---------------------------------------------------------------- import

type Attributes = HashMap<String, String>;

fn attributes(element: &quick_xml::events::BytesStart<'_>) -> Result<Attributes, ExportError> {
    let mut map = HashMap::new();
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|e| ExportError::Flame(e.to_string()))?;
        let key = attribute.key.as_ref().to_ascii_lowercase();
        let value = attribute
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|e| ExportError::Flame(e.to_string()))?;
        map.insert(key, value.into_owned());
    }
    Ok(map)
}

fn numbers(value: &str) -> Vec<f32> {
    value
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect()
}

/// Camera values are `f64` in our model: parse them at full precision so
/// our own files round-trip exactly.
fn numbers64(value: &str) -> Vec<f64> {
    value
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect()
}

fn number64(attrs: &Attributes, key: &str) -> Option<f64> {
    attrs.get(key).and_then(|v| v.trim().parse().ok())
}

fn number(attrs: &Attributes, key: &str) -> Option<f32> {
    attrs.get(key).and_then(|v| v.trim().parse().ok())
}

/// One of our own attributes: `calyptra_{key}`, or `fractals_{key}` from
/// files written before the rename.
fn ours<'a>(attrs: &'a Attributes, key: &str) -> Option<&'a str> {
    attrs
        .get(&format!("calyptra_{key}"))
        .or_else(|| attrs.get(&format!("fractals_{key}")))
        .map(String::as_str)
}

/// flam3 `coefs`/`post` order (a d b e c f) → our affine.
fn affine_from_coefs(value: &str) -> Option<Affine> {
    match numbers(value)[..] {
        [a, d, b, e, c, f] => Some(Affine::planar(a, b, c, d, e, f)),
        _ => None,
    }
}

struct FlameBuilder {
    attrs: Attributes,
    xforms: Vec<Attributes>,
    final_xform: Option<Attributes>,
    palette_hex: String,
    palette_colors: Vec<(usize, [u8; 3])>,
}

pub(crate) fn parse_flames(text: &str) -> Result<Vec<ImportedFlame>, ExportError> {
    let mut reader = quick_xml::Reader::from_str(text);
    let mut flames = Vec::new();
    let mut current: Option<FlameBuilder> = None;
    let mut in_palette = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| ExportError::Flame(format!("XML error: {e}")))?;
        match event {
            Event::Start(ref element) | Event::Empty(ref element) => {
                let tag = element.local_name().as_ref().to_ascii_lowercase();
                let is_empty = matches!(event, Event::Empty(_));
                match (tag.as_str(), current.as_mut()) {
                    ("flame", _) => {
                        current = Some(FlameBuilder {
                            attrs: attributes(element)?,
                            xforms: Vec::new(),
                            final_xform: None,
                            palette_hex: String::new(),
                            palette_colors: Vec::new(),
                        });
                    }
                    ("xform", Some(builder)) => builder.xforms.push(attributes(element)?),
                    ("finalxform", Some(builder)) => {
                        builder.final_xform = Some(attributes(element)?)
                    }
                    ("palette", Some(_)) => in_palette = !is_empty,
                    ("color", Some(builder)) => {
                        let attrs = attributes(element)?;
                        let index = number(&attrs, "index").unwrap_or(0.0) as usize;
                        if let [r, g, b] =
                            attrs.get("rgb").map(|v| numbers(v)).unwrap_or_default()[..]
                        {
                            let byte = |c: f32| c.round().clamp(0.0, 255.0) as u8;
                            builder
                                .palette_colors
                                .push((index, [byte(r), byte(g), byte(b)]));
                        }
                    }
                    _ => {}
                }
            }
            Event::Text(ref text) if in_palette => {
                if let Some(builder) = current.as_mut() {
                    builder.palette_hex.push_str(text.as_ref());
                }
            }
            Event::End(ref element) => {
                let tag = element.local_name().as_ref().to_ascii_lowercase();
                match tag.as_str() {
                    "palette" => in_palette = false,
                    "flame" => {
                        if let Some(builder) = current.take() {
                            flames.push(build(builder));
                        }
                    }
                    _ => {}
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if flames.is_empty() {
        return Err(ExportError::Flame("no <flame> found in the file".into()));
    }
    Ok(flames)
}

fn variation_by_name(name: &str) -> Option<VariationKind> {
    VariationKind::ALL
        .into_iter()
        .find(|k| k.name().eq_ignore_ascii_case(name))
}

/// Our brightness per unit of Apophysis brightness (at zoom 0).
const APOPHYSIS_BRIGHTNESS: f64 = 1.4;

/// Attributes of an `<xform>` that are not variations.
const XFORM_KEYS: &[&str] = &[
    "weight",
    "color",
    "symmetry",
    "color_speed",
    "coefs",
    "post",
    "opacity",
    "name",
    "animate",
    "var_color",
    "chaos",
    "plotmode",
    "visible",
    "calyptra_z",
    "calyptra_post_z",
    "fractals_z",
    "fractals_post_z",
    // JWildfire extras.
    "color_type",
    "material",
    "material_speed",
    "mod_gamma",
    "mod_gamma_speed",
    "mod_contrast",
    "mod_contrast_speed",
    "mod_saturation",
    "mod_saturation_speed",
    "mod_hue",
    "mod_hue_speed",
];

fn build_xform(attrs: &Attributes, label: &str, warnings: &mut Vec<String>) -> Xform {
    let mut xform = Xform {
        weight: number(attrs, "weight").unwrap_or(0.5),
        color: number(attrs, "color").unwrap_or(0.0).clamp(0.0, 1.0),
        color_speed: number(attrs, "color_speed")
            .unwrap_or_else(|| (1.0 - number(attrs, "symmetry").unwrap_or(0.0)) * 0.5)
            .clamp(0.0, 1.0),
        affine: attrs
            .get("coefs")
            .and_then(|v| affine_from_coefs(v))
            .unwrap_or(Affine::IDENTITY),
        variations: Vec::new(),
        post: attrs.get("post").and_then(|v| affine_from_coefs(v)),
    };
    // Our own 3D terms.
    let apply_z = |affine: &mut Affine, value: &str| {
        if let [xz, yz, zx, zy, zz, zc] = numbers(value)[..] {
            (
                affine.xz, affine.yz, affine.zx, affine.zy, affine.zz, affine.zc,
            ) = (xz, yz, zx, zy, zz, zc);
        }
    };
    if let Some(z) = ours(attrs, "z") {
        apply_z(&mut xform.affine, z);
    }
    if let (Some(z), Some(post)) = (ours(attrs, "post_z"), xform.post.as_mut()) {
        apply_z(post, z);
    }
    if xform.post == Some(Affine::IDENTITY) {
        xform.post = None;
    }

    let mut variations: Vec<Variation> = Vec::new();
    let mut unknown = Vec::new();
    for (key, value) in attrs {
        if XFORM_KEYS.contains(&key.as_str()) {
            continue;
        }
        let kind = variation_by_name(key);
        // A parameter (`julian_power`) belongs to a variation the transform
        // uses; other names with '_' (`pre_blur`) are variations themselves.
        let is_param = kind.is_none()
            && key
                .match_indices('_')
                .any(|(i, _)| attrs.contains_key(&key[..i]));
        if is_param {
            continue;
        }
        let Ok(weight) = value.trim().parse::<f32>() else {
            continue;
        };
        if weight == 0.0 {
            continue;
        }
        match kind {
            Some(kind) => {
                let mut variation = Variation::new(kind, weight);
                for (slot, (param, _)) in variation.params.iter_mut().zip(kind.params()) {
                    if let Some(v) = number(
                        attrs,
                        &format!("{}_{param}", kind.name().to_ascii_lowercase()),
                    ) {
                        *slot = v;
                    }
                }
                variations.push(variation);
            }
            None => unknown.push(format!("{key} ({weight})")),
        }
    }
    unknown.sort();
    if !unknown.is_empty() {
        warnings.push(format!(
            "{label}: unsupported variations skipped: {}",
            unknown.join(", ")
        ));
    }
    // Deterministic order: largest weight first, then name.
    variations.sort_by(|a, b| {
        b.weight
            .abs()
            .total_cmp(&a.weight.abs())
            .then(a.kind.name().cmp(b.kind.name()))
    });
    if variations.len() > Xform::MAX_VARIATIONS {
        let dropped: Vec<String> = variations[Xform::MAX_VARIATIONS..]
            .iter()
            .map(|v| v.kind.name().to_owned())
            .collect();
        warnings.push(format!(
            "{label}: only {} variations per transform; dropped the weakest: {}",
            Xform::MAX_VARIATIONS,
            dropped.join(", ")
        ));
        variations.truncate(Xform::MAX_VARIATIONS);
    }
    if variations.is_empty() {
        variations.push(Variation::new(VariationKind::Linear, 1.0));
    }
    xform.variations = variations;
    xform
}

/// Post-affine that mirrors y (flam3's y points down, ours up).
fn flip_y(affine: Affine) -> Affine {
    Affine {
        d: -affine.d,
        e: -affine.e,
        f: -affine.f,
        yz: -affine.yz,
        ..affine
    }
}

/// Mirrors the flame's output in y, exactly: through the final transform.
/// Applying it twice restores the original (an added mirror-only final
/// transform is removed again).
fn mirror_y(flame: &mut Flame) {
    let mirror_only = |x: &Xform| {
        x.color_speed == 0.0
            && x.affine == Affine::IDENTITY
            && x.variations == [Variation::new(VariationKind::Linear, 1.0)]
            && x.post == Some(flip_y(Affine::IDENTITY))
    };
    match flame.final_xform.as_mut() {
        Some(fin) if mirror_only(fin) => flame.final_xform = None,
        Some(fin) => {
            let post = flip_y(fin.post.unwrap_or(Affine::IDENTITY));
            fin.post = (post != Affine::IDENTITY).then_some(post);
        }
        None => {
            flame.final_xform = Some(Xform {
                weight: 0.0,
                color: 0.0,
                color_speed: 0.0,
                affine: Affine::IDENTITY,
                variations: vec![Variation::new(VariationKind::Linear, 1.0)],
                post: Some(flip_y(Affine::IDENTITY)),
            });
        }
    }
}

fn build(builder: FlameBuilder) -> ImportedFlame {
    let attrs = &builder.attrs;
    let name = attrs
        .get("name")
        .cloned()
        .unwrap_or_else(|| "Imported flame".into());
    let mut warnings = Vec::new();

    let mut xforms: Vec<Xform> = builder
        .xforms
        .iter()
        .enumerate()
        .map(|(i, x)| build_xform(x, &format!("transform {}", i + 1), &mut warnings))
        .collect();
    if xforms.len() > Flame::MAX_XFORMS {
        warnings.push(format!(
            "only {} transforms are supported; {} dropped",
            Flame::MAX_XFORMS,
            xforms.len() - Flame::MAX_XFORMS
        ));
        xforms.truncate(Flame::MAX_XFORMS);
    }
    if xforms.is_empty() {
        warnings.push("the flame has no transforms".into());
        xforms.push(Xform::default());
    }
    let final_xform = builder
        .final_xform
        .as_ref()
        .map(|x| build_xform(x, "final transform", &mut warnings));

    // Camera: flam3 maps `scale` pixels per unit at `size`, times 2^zoom.
    let height = match attrs.get("size").map(|v| numbers64(v)).unwrap_or_default()[..] {
        [_, h] => h.max(1.0),
        _ => 600.0,
    };
    let scale = number64(attrs, "scale").unwrap_or(height / 4.0);
    let zoom = scale * 2f64.powf(number64(attrs, "zoom").unwrap_or(0.0)) * 2.0 / height;
    let center = match attrs
        .get("center")
        .map(|v| numbers64(v))
        .unwrap_or_default()[..]
    {
        // y mirrored, like the flame itself.
        [x, y] => [x, -y],
        _ => [0.0, 0.0],
    };
    let mut camera = FlameCamera {
        center,
        zoom,
        rotation_degrees: -number64(attrs, "rotate").unwrap_or(0.0),
        ..FlameCamera::default()
    };
    if let Some(extra) = ours(attrs, "camera") {
        if let [yaw, pitch, perspective, dof, focus, fade, exact_zoom] = numbers64(extra)[..] {
            // Exact: `scale` → zoom arithmetic can be off in the last bit.
            camera.zoom = exact_zoom;
            camera.yaw_degrees = yaw;
            camera.pitch_degrees = pitch;
            camera.perspective = perspective;
            camera.depth_of_field = dof;
            camera.focus_depth = focus;
            camera.depth_fade = fade;
        }
    } else {
        // Apophysis 3D hack / JWildfire camera: angles in radians.
        camera.pitch_degrees = number64(attrs, "cam_pitch").unwrap_or(0.0).to_degrees();
        camera.yaw_degrees = number64(attrs, "cam_yaw").unwrap_or(0.0).to_degrees();
        camera.perspective = number64(attrs, "cam_perspective")
            .or_else(|| number64(attrs, "cam_persp"))
            .unwrap_or(0.0);
        camera.depth_of_field = number64(attrs, "cam_dof").unwrap_or(0.0) * 0.1;
    }

    let background = match attrs
        .get("background")
        .map(|v| numbers(v))
        .unwrap_or_default()[..]
    {
        [r, g, b] => [r, g, b].map(|c| color::srgb_to_linear(c.clamp(0.0, 1.0))),
        _ => [0.0; 3],
    };
    let mut flame = Flame {
        xforms,
        final_xform,
        camera,
        // Other programs' `quality` is usually a preview setting.
        quality: match ours(attrs, "camera") {
            Some(_) => number(attrs, "quality").map_or(400.0, |q| q.clamp(20.0, 5000.0)),
            None => 400.0,
        },
        supersample: number(attrs, "oversample").map_or(2, |s| s.clamp(1.0, 3.0) as u32),
        brightness: ours(attrs, "brightness").and_then(|v| v.trim().parse().ok()).unwrap_or_else(|| {
            // Apophysis draws 4^zoom times the samples at a zoom without
            // normalizing them away, and its brightness runs hotter than
            // ours: fitted against Apophysis renders.
            let zoom = number64(attrs, "zoom").unwrap_or(0.0);
            let file = number64(attrs, "brightness").unwrap_or(4.0);
            (file * APOPHYSIS_BRIGHTNESS * 2f64.powf(zoom)) as f32
        }),
        gamma: number(attrs, "gamma").unwrap_or(4.0),
        vibrancy: number(attrs, "vibrancy").unwrap_or(1.0),
        // flam3's density estimation (its defaults when absent).
        estimator_radius: number(attrs, "estimator_radius").unwrap_or(9.0),
        estimator_minimum: number(attrs, "estimator_minimum").unwrap_or(0.0),
        estimator_curve: number(attrs, "estimator_curve").unwrap_or(0.4),
        // Apophysis 3D hack carries z through 2D variations; JWildfire only
        // with `preserve_z`.
        preserve_z: match attrs.get("preserve_z") {
            Some(v) => v.trim() == "1",
            None => !attrs
                .get("version")
                .is_some_and(|v| v.to_ascii_lowercase().contains("jwildfire")),
        },
        background,
    };
    mirror_y(&mut flame);

    let gradient = palette(&builder).unwrap_or_else(|| {
        warnings.push("no palette found; using the default".into());
        color::Gradient::default()
    });
    ImportedFlame {
        name,
        flame,
        gradient,
        warnings,
    }
}

fn palette(builder: &FlameBuilder) -> Option<color::Gradient> {
    let mut colors: Vec<[u8; 3]> = if builder.palette_colors.is_empty() {
        // Apophysis: 256 colors as hex, 6 (RGB) or 8 (ARGB) digits each.
        let hex: Vec<u8> = builder
            .palette_hex
            .bytes()
            .filter(u8::is_ascii_hexdigit)
            .collect();
        let per_color = if hex.len() >= 256 * 8 { 8 } else { 6 };
        hex.chunks_exact(per_color)
            .filter_map(|chunk| {
                let digits = std::str::from_utf8(&chunk[per_color - 6..]).ok()?;
                let value = u32::from_str_radix(digits, 16).ok()?;
                Some([(value >> 16) as u8, (value >> 8) as u8, value as u8])
            })
            .collect()
    } else {
        let mut indexed = builder.palette_colors.clone();
        indexed.sort_by_key(|(i, _)| *i);
        indexed.into_iter().map(|(_, c)| c).collect()
    };
    colors.truncate(256);
    if colors.len() < 2 {
        return None;
    }
    let last = (colors.len() - 1) as f32;
    let mut gradient = color::Gradient {
        stops: colors
            .into_iter()
            .enumerate()
            .map(|(i, c)| color::Stop::new(i as f32 / last, c))
            .collect(),
    };
    gradient.simplify(0.008);
    Some(gradient)
}

// ---------------------------------------------------------------- export

fn coefs(affine: &Affine) -> String {
    let Affine {
        a, b, c, d, e, f, ..
    } = *affine;
    format!("{a} {d} {b} {e} {c} {f}")
}

fn z_terms(affine: &Affine) -> String {
    format!(
        "{} {} {} {} {} {}",
        affine.xz, affine.yz, affine.zx, affine.zy, affine.zz, affine.zc
    )
}

fn xform_xml(out: &mut String, tag: &str, xform: &Xform, with_weight: bool) {
    let _ = write!(out, "  <{tag}");
    if with_weight {
        let _ = write!(out, " weight=\"{}\"", xform.weight);
    }
    let _ = write!(
        out,
        " color=\"{}\" color_speed=\"{}\" symmetry=\"{}\"",
        xform.color,
        xform.color_speed,
        1.0 - 2.0 * xform.color_speed
    );
    for variation in &xform.variations {
        let name = variation.kind.name();
        let _ = write!(out, " {name}=\"{}\"", variation.weight);
        for ((param, _), value) in variation.kind.params().iter().zip(variation.params) {
            let _ = write!(out, " {name}_{param}=\"{value}\"");
        }
    }
    let _ = write!(out, " coefs=\"{}\"", coefs(&xform.affine));
    if xform.affine.is_3d() {
        let _ = write!(out, " calyptra_z=\"{}\"", z_terms(&xform.affine));
    }
    if let Some(post) = &xform.post {
        let _ = write!(out, " post=\"{}\"", coefs(post));
        if post.is_3d() {
            let _ = write!(out, " calyptra_post_z=\"{}\"", z_terms(post));
        }
    }
    let _ = writeln!(out, "/>");
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub(crate) fn flame_to_xml(name: &str, flame: &Flame, gradient: &color::Gradient) -> String {
    // Back into flam3's y-down convention.
    let mut flame = flame.clone();
    mirror_y(&mut flame);
    let camera = &flame.camera;
    let (width, height) = (1024.0, 768.0);
    let scale = camera.zoom * height / 2.0;
    let background = flame.background.map(color::linear_to_srgb);

    let mut out = String::new();
    let _ = writeln!(
        out,
        "<flame name=\"{}\" version=\"Calyptra\" size=\"{width} {height}\" center=\"{} {}\" \
         scale=\"{scale}\" rotate=\"{}\" oversample=\"{}\" quality=\"{}\" \
         background=\"{} {} {}\" brightness=\"{}\" calyptra_brightness=\"{}\" gamma=\"{}\" vibrancy=\"{}\" \
         estimator_radius=\"{}\" estimator_minimum=\"{}\" estimator_curve=\"{}\" \
         cam_pitch=\"{}\" cam_yaw=\"{}\" cam_perspective=\"{}\" cam_dof=\"{}\" preserve_z=\"{}\" \
         calyptra_camera=\"{} {} {} {} {} {} {}\">",
        escape(name),
        camera.center[0],
        -camera.center[1],
        -camera.rotation_degrees,
        flame.supersample,
        flame.quality,
        background[0],
        background[1],
        background[2],
        f64::from(flame.brightness) / APOPHYSIS_BRIGHTNESS,
        flame.brightness,
        flame.gamma,
        flame.vibrancy,
        flame.estimator_radius,
        flame.estimator_minimum,
        flame.estimator_curve,
        camera.pitch_degrees.to_radians(),
        camera.yaw_degrees.to_radians(),
        camera.perspective,
        camera.depth_of_field * 10.0,
        u8::from(flame.preserve_z),
        camera.yaw_degrees,
        camera.pitch_degrees,
        camera.perspective,
        camera.depth_of_field,
        camera.focus_depth,
        camera.depth_fade,
        camera.zoom,
    );
    for xform in &flame.xforms {
        xform_xml(&mut out, "xform", xform, true);
    }
    if let Some(fin) = &flame.final_xform {
        xform_xml(&mut out, "finalxform", fin, false);
    }
    let _ = writeln!(out, "  <palette count=\"256\" format=\"RGB\">");
    let colors = gradient.bake_srgb8(256);
    for row in colors.chunks(8) {
        let line: String = row
            .iter()
            .map(|[r, g, b]| format!("{r:02X}{g:02X}{b:02X}"))
            .collect();
        let _ = writeln!(out, "    {line}");
    }
    let _ = writeln!(out, "  </palette>");
    let _ = writeln!(out, "</flame>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small flam3 file in the style Apophysis writes.
    const APOPHYSIS: &str = r#"<Flames name="test">
<flame name="Two swirls" version="Apophysis 2.09" size="800 600" center="0.5 -0.25" scale="150" oversample="1" quality="50" background="0 0 0" brightness="4" gamma="4" vibrancy="1" rotate="30">
   <xform weight="0.5" color="0" symmetry="0.5" linear="0.25" swirl="0.75" coefs="1 0 0 1 0.5 0"/>
   <xform weight="1" color="1" spherical="1" julian="0.5" julian_power="4" julian_dist="-1" fan2="0.3" coefs="0.5 0.2 -0.2 0.5 0 1" post="1 0 0 1 0 0.1"/>
   <finalxform color="0" symmetry="1" linear="1" coefs="1 0 0 1 0 0"/>
   <palette count="256" format="RGB">
      FF000000FF00
   </palette>
</flame>
<flame name="Plain"><xform weight="1" color="0" linear="1" coefs="0.5 0 0 0.5 0 0"/>
<color index="0" rgb="255 0 0"/><color index="255" rgb="0 0 255"/></flame>
</Flames>"#;

    #[test]
    fn apophysis_flames_import() {
        let flames = parse_flames(APOPHYSIS).unwrap();
        assert_eq!(flames.len(), 2);
        let first = &flames[0];
        assert_eq!(first.name, "Two swirls");
        let flame = &first.flame;
        assert_eq!(flame.xforms.len(), 2);

        // Column-wise coefs: "1 0 0 1 0.5 0" is x' = x + 0.5.
        let t0 = &flame.xforms[0];
        assert_eq!(t0.affine, Affine::planar(1.0, 0.0, 0.5, 0.0, 1.0, 0.0));
        assert_eq!(t0.color_speed, 0.25); // symmetry 0.5 → (1 − 0.5) / 2
        assert_eq!(t0.variations[0].kind, VariationKind::Swirl); // largest first
        assert_eq!(t0.variations.len(), 2);

        let t1 = &flame.xforms[1];
        let julian = t1
            .variations
            .iter()
            .find(|v| v.kind == VariationKind::Julian)
            .unwrap();
        assert_eq!(julian.params[..2], [4.0, -1.0]);
        assert!(t1.post.is_some());
        // fan2 is not supported: warned, not silently dropped.
        assert!(
            first.warnings.iter().any(|w| w.contains("fan2")),
            "{:?}",
            first.warnings
        );

        // The existing final transform carries the y mirror in its post.
        let fin = flame.final_xform.as_ref().unwrap();
        assert_eq!(fin.post.unwrap().e, -1.0);
        assert_eq!(flame.camera.center, [0.5, 0.25]);
        assert!((flame.camera.zoom - 150.0 * 2.0 / 600.0).abs() < 1e-9);

        // Hex palette: red then green.
        assert_eq!(first.gradient.sample_srgb8(0.0), [255, 0, 0]);
        assert_eq!(first.gradient.sample_srgb8(1.0), [0, 255, 0]);

        // The second flame had no final transform: a mirror-only one is added.
        let second = &flames[1];
        assert!(second.flame.final_xform.is_some());
        assert_eq!(second.gradient.sample_srgb8(0.0), [255, 0, 0]);
        assert_eq!(second.gradient.sample_srgb8(1.0), [0, 0, 255]);
    }

    #[test]
    fn our_flames_round_trip() {
        for (name, flame) in scene::flame::presets() {
            let gradient = color::Gradient::default();
            let xml = flame_to_xml(name, &flame, &gradient);
            let back = parse_flames(&xml).unwrap().remove(0);
            assert_eq!(back.name, name);
            assert!(back.warnings.is_empty(), "{name}: {:?}", back.warnings);
            let (a, b) = (&back.flame, &flame);
            assert_eq!(a.xforms.len(), b.xforms.len(), "{name}");
            for (x, y) in a.xforms.iter().zip(&b.xforms) {
                assert!((x.color_speed - y.color_speed).abs() < 1e-6, "{name}");
                assert_eq!(x.affine, y.affine, "{name}");
                assert_eq!(x.variations.len(), y.variations.len(), "{name}");
            }
            // The mirror applied on export is undone on import.
            assert_eq!(a.final_xform, b.final_xform, "{name}");
            assert_eq!(a.camera, b.camera, "{name}");
        }
    }

    #[test]
    fn files_from_before_the_rename_still_load() {
        let gradient = color::Gradient::default();
        for (name, flame) in scene::flame::presets() {
            let current = flame_to_xml(name, &flame, &gradient);
            let legacy = current.replace("calyptra_", "fractals_");
            assert_ne!(legacy, current, "{name}");
            let (a, b) = (parse_flames(&current).unwrap(), parse_flames(&legacy).unwrap());
            assert_eq!(a[0].flame, b[0].flame, "{name}");
            assert!(b[0].warnings.is_empty(), "{name}: {:?}", b[0].warnings);
        }
    }

    #[test]
    fn mirroring_twice_restores_the_flame() {
        let mut flame = scene::flame::presets().remove(0).1;
        let original = flame.clone();
        mirror_y(&mut flame);
        assert_ne!(flame, original);
        mirror_y(&mut flame);
        assert_eq!(flame, original);
    }

    #[test]
    fn files_without_flames_are_an_error() {
        assert!(parse_flames("<notaflame/>").is_err());
        assert!(parse_flames("").is_err());
    }
}
