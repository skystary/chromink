//! Boundary-band bilateral measurement source. Classification always uses the unfiltered raster.
use crate::control::WorkControl;
use image::RgbaImage;
use rayon::prelude::*;
use std::sync::OnceLock;
static RANGE_WEIGHTS: OnceLock<Vec<f64>> = OnceLock::new();
fn dilate(mask: &[u8], w: usize, h: usize, horizontal: bool) -> Vec<u8> {
    let mut out = vec![0; mask.len()];
    let (lines, len, stride) = if horizontal { (h, w, 1) } else { (w, h, w) };
    let mut dist = vec![0usize; len];
    for l in 0..lines {
        let base = if horizontal { l * w } else { l };
        let mut d = len;
        for p in 0..len {
            d = if mask[base + p * stride] > 0 {
                0
            } else {
                d + 1
            };
            dist[p] = d;
        }
        d = len;
        for p in (0..len).rev() {
            d = if mask[base + p * stride] > 0 {
                0
            } else {
                d + 1
            };
            dist[p] = dist[p].min(d);
        }
        for p in 0..len {
            out[base + p * stride] = u8::from(dist[p] <= 12);
        }
    }
    out
}
pub(super) fn run(img: &RgbaImage, labels: &[i32], c: &WorkControl) -> Result<RgbaImage, String> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let _scratch = c.reserve(w * h * 8)?;
    let mut seed = vec![0; w * h];
    for y in 0..h {
        c.check()?;
        for x in 0..w {
            let i = y * w + x;
            seed[i] = u8::from(
                x > 0 && labels[i - 1] != labels[i]
                    || x + 1 < w && labels[i + 1] != labels[i]
                    || y > 0 && labels[i - w] != labels[i]
                    || y + 1 < h && labels[i + w] != labels[i],
            );
        }
    }
    let band = dilate(&dilate(&seed, w, h, true), w, h, false);
    let spatial: Vec<_> = (-4i32..=4)
        .flat_map(|dy| {
            (-4i32..=4).map(move |dx| (-((dx * dx + dy * dy) as f64) / (2. * 1.8 * 1.8)).exp())
        })
        .collect();
    let range = RANGE_WEIGHTS.get_or_init(|| {
        (0..=4 * 255 * 255)
            .map(|sq| (-(sq as f64) / (2. * 60. * 60.)).exp())
            .collect()
    });
    let mut out = img.clone();
    out.as_mut()
        .par_chunks_mut(w * 4)
        .enumerate()
        .try_for_each(|(y, row)| {
            c.check()?;
            for x in 0..w {
                if band[y * w + x] == 0 {
                    continue;
                }
                let p = img.get_pixel(x as u32, y as u32);
                let mut sum = [0.; 4];
                let mut total = 0.;
                for ny in y.saturating_sub(4)..=(h - 1).min(y + 4) {
                    for nx in x.saturating_sub(4)..=(w - 1).min(x + 4) {
                        let q = img.get_pixel(nx as u32, ny as u32);
                        let sq = (0..4)
                            .map(|ch| {
                                let d = q[ch] as i32 - p[ch] as i32;
                                (d * d) as usize
                            })
                            .sum::<usize>();
                        let weight = spatial[(ny + 4 - y) * 9 + nx + 4 - x] * range[sq];
                        for ch in 0..4 {
                            sum[ch] += weight * q[ch] as f64;
                        }
                        total += weight;
                    }
                }
                row[x * 4..x * 4 + 4].copy_from_slice(
                    &sum.map(|v| (v / total).round_ties_even().clamp(0., 255.) as u8),
                );
            }
            Ok::<_, String>(())
        })?;
    Ok(out)
}
