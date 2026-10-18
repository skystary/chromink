//! Sample only the visible region of the cached original, bounded to four megapixels.
use super::Geometry;
use chromink_core::{control::WorkControl, parallel::Parallelism};
use image::RgbaImage;
pub fn viewport(
    source: &RgbaImage,
    g: &Geometry,
    workers: &Parallelism,
    control: &WorkControl,
) -> Result<RgbaImage, String> {
    if g.width == 0
        || g.height == 0
        || u64::from(g.width) * u64::from(g.height) > 4_194_304
        || ![g.x, g.y, g.stage_width, g.stage_height]
            .iter()
            .all(|v| v.is_finite())
        || g.stage_width <= 0.
        || g.stage_height <= 0.
    {
        return Err("原图预览尺寸无效".into());
    }
    let _memory = control.reserve(g.width as usize * g.height as usize * 12)?;
    let chunks: Vec<_> = (0..g.height).step_by(32).collect();
    let rows = workers.map(&chunks, |start| -> Result<Vec<u8>, String> {
        control.check()?;
        let end = (*start + 32).min(g.height);
        let mut bytes = vec![0; (end - *start) as usize * g.width as usize * 4];
        let sx = source.width() as f32 / g.stage_width;
        let sy = source.height() as f32 / g.stage_height;
        for y in *start..end {
            control.check()?;
            let py = (y as f32 + 0.5 - g.y) * sy;
            if py < 0. || py >= source.height() as f32 {
                continue;
            }
            for x in 0..g.width {
                let px = (x as f32 + 0.5 - g.x) * sx;
                if px < 0. || px >= source.width() as f32 {
                    continue;
                }
                let index = ((y - *start) as usize * g.width as usize + x as usize) * 4;
                if sx <= 1. && sy <= 1. {
                    bytes[index..index + 4]
                        .copy_from_slice(&source.get_pixel(px as u32, py as u32).0);
                } else {
                    let fx = px - 0.5;
                    let fy = py - 0.5;
                    let ax = fx.floor();
                    let ay = fy.floor();
                    let dx = fx - ax;
                    let dy = fy - ay;
                    let mut rgba = [0f32; 4];
                    for (ox, oy, weight) in [
                        (0., 0., (1. - dx) * (1. - dy)),
                        (1., 0., dx * (1. - dy)),
                        (0., 1., (1. - dx) * dy),
                        (1., 1., dx * dy),
                    ] {
                        let p = source.get_pixel(
                            (ax + ox).clamp(0., (source.width() - 1) as f32) as u32,
                            (ay + oy).clamp(0., (source.height() - 1) as f32) as u32,
                        );
                        let alpha = p[3] as f32 * weight;
                        rgba[3] += alpha;
                        for c in 0..3 {
                            rgba[c] += p[c] as f32 * alpha;
                        }
                    }
                    if rgba[3] > 0. {
                        for c in 0..3 {
                            bytes[index + c] = (rgba[c] / rgba[3]).round() as u8;
                        }
                        bytes[index + 3] = rgba[3].round() as u8;
                    }
                }
            }
        }
        Ok(bytes)
    });
    let mut bytes = Vec::with_capacity(g.width as usize * g.height as usize * 4);
    for row in rows {
        bytes.extend(row?);
    }
    control.check()?;
    RgbaImage::from_raw(g.width, g.height, bytes).ok_or_else(|| "原图预览像素格式无效".into())
}
