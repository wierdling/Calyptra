use std::io::BufWriter;
use std::path::Path;

use render::{HdrImage, display_transform};
use scene::{DisplaySettings, Scene};

use crate::ExportError;
use crate::scene_file::{PNG_SCENE_KEY, scene_to_json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    /// Tone-mapped, 8 bits per channel, dithered.
    Png8,
    /// Tone-mapped, 16 bits per channel: headroom for editing without banding.
    Png16,
    /// Linear HDR (exposure applied, no tone map) for grading elsewhere.
    Exr,
}

impl ImageFormat {
    pub const ALL: [Self; 3] = [Self::Png8, Self::Png16, Self::Exr];

    pub fn label(self) -> &'static str {
        match self {
            Self::Png8 => "PNG (8-bit)",
            Self::Png16 => "PNG (16-bit)",
            Self::Exr => "OpenEXR (linear HDR)",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Png8 | Self::Png16 => "png",
            Self::Exr => "exr",
        }
    }
}

/// Writes `image` to `path`. PNGs embed `scene` so the image can be
/// reopened as a scene.
pub fn save_image(
    path: &Path,
    image: &HdrImage,
    format: ImageFormat,
    scene: &Scene,
) -> Result<(), ExportError> {
    match format {
        ImageFormat::Png8 => write_png(path, image, png::BitDepth::Eight, scene, false),
        ImageFormat::Png16 => write_png(path, image, png::BitDepth::Sixteen, scene, false),
        ImageFormat::Exr => write_exr(path, image, &scene.display),
    }
}

/// Writes a 16-bit movie frame: fast compression, since frames are
/// intermediate files and saving must not dominate render time.
pub(crate) fn save_frame(path: &Path, image: &HdrImage, scene: &Scene) -> Result<(), ExportError> {
    write_png(path, image, png::BitDepth::Sixteen, scene, true)
}

fn write_png(
    path: &Path,
    image: &HdrImage,
    depth: png::BitDepth,
    scene: &Scene,
    fast: bool,
) -> Result<(), ExportError> {
    let display = &scene.display;
    let png_err = |e: png::EncodingError| ExportError::Png(e.to_string());
    let file = BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, image.width, image.height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(depth);
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    if fast {
        encoder.set_compression(png::Compression::Fast);
    }
    encoder
        .add_itxt_chunk(PNG_SCENE_KEY.to_owned(), scene_to_json(scene)?)
        .map_err(png_err)?;
    encoder
        .add_text_chunk("Software".to_owned(), "Fractals".to_owned())
        .map_err(png_err)?;
    let mut writer = encoder.write_header().map_err(png_err)?;

    let pixel_count = (image.width * image.height) as usize;
    let data = match depth {
        png::BitDepth::Sixteen => {
            let mut data = Vec::with_capacity(pixel_count * 6);
            for &[r, g, b, _] in &image.pixels {
                for c in display_transform([r, g, b], display) {
                    data.extend_from_slice(&((c * 65535.0).round() as u16).to_be_bytes());
                }
            }
            data
        }
        _ => {
            let mut data = Vec::with_capacity(pixel_count * 3);
            for (i, &[r, g, b, _]) in image.pixels.iter().enumerate() {
                let noise = if display.dither {
                    triangular_noise(i as u32)
                } else {
                    0.0
                };
                for c in display_transform([r, g, b], display) {
                    data.push((c * 255.0 + noise).round().clamp(0.0, 255.0) as u8);
                }
            }
            data
        }
    };
    writer.write_image_data(&data).map_err(png_err)?;
    writer.finish().map_err(png_err)?;
    Ok(())
}

fn write_exr(path: &Path, image: &HdrImage, display: &DisplaySettings) -> Result<(), ExportError> {
    let exposure = display.exposure_ev.exp2();
    let width = image.width as usize;
    exr::prelude::write_rgb_file(path, width, image.height as usize, |x, y| {
        let [r, g, b, _] = image.pixels[y * width + x];
        (r * exposure, g * exposure, b * exposure)
    })
    .map_err(|e| ExportError::Exr(e.to_string()))
}

/// Deterministic triangular-distributed noise in (-1, 1) LSB: the sum of
/// two uniform hashes, which hides 8-bit banding without visible pattern.
fn triangular_noise(index: u32) -> f32 {
    fn hash(mut h: u32) -> f32 {
        h = (h ^ (h >> 16)).wrapping_mul(0x7feb_352d);
        h = (h ^ (h >> 15)).wrapping_mul(0x846c_a68b);
        h ^= h >> 16;
        h as f32 / u32::MAX as f32
    }
    hash(index.wrapping_mul(2)) + hash(index.wrapping_mul(2) + 1) - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_image() -> HdrImage {
        // A horizontal ramp from black past white (HDR).
        let (width, height) = (64, 4);
        let pixels = (0..width * height)
            .map(|i| {
                let v = (i % width) as f32 / 16.0;
                [v, v * 0.5, v * 0.25, 1.0]
            })
            .collect();
        HdrImage {
            width,
            height,
            pixels,
        }
    }

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "fractals-export-test-{}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn png16_round_trips_through_the_decoder() {
        let path = temp_path("a.png");
        let scene = Scene::default();
        save_image(&path, &test_image(), ImageFormat::Png16, &scene).unwrap();

        let decoder =
            png::Decoder::new(std::io::BufReader::new(std::fs::File::open(&path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        assert_eq!(reader.info().bit_depth, png::BitDepth::Sixteen);
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let frame = reader.next_frame(&mut buf).unwrap();
        assert_eq!((frame.width, frame.height), (64, 4));
        // First pixel is black, the ramp brightens left to right.
        let red = |x: usize| u16::from_be_bytes([buf[x * 6], buf[x * 6 + 1]]);
        assert_eq!(red(0), 0);
        assert!(red(10) < red(20) && red(20) < red(63));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn exr_is_written() {
        let path = temp_path("b.exr");
        save_image(&path, &test_image(), ImageFormat::Exr, &Scene::default()).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn dither_noise_is_small_and_centered() {
        let values: Vec<f32> = (0..10_000).map(triangular_noise).collect();
        assert!(values.iter().all(|v| v.abs() < 1.0));
        let mean = values.iter().sum::<f32>() / values.len() as f32;
        assert!(mean.abs() < 0.02, "{mean}");
    }
}
