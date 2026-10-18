//! Vecline threshold.ts: Rec.709 Otsu and Bradley–Roth local thresholding.
//! MIT provenance: documentation/algorithms/VECLINE-LICENSE.txt.
use crate::{control::WorkControl, TraceOptions, TRANSPARENT};
use image::RgbaImage;
fn luma(p: &[u8]) -> f64 {
    0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64
}
pub(crate) fn otsu(image: &RgbaImage) -> u8 {
    let mut hist = [0u64; 256];
    for p in image.pixels().filter(|p| p[3] >= 128) {
        hist[luma(&p.0).round() as usize] += 1;
    }
    let total: u64 = hist.iter().sum();
    if total == 0 {
        return 128;
    }
    let sum: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &v)| i as f64 * v as f64)
        .sum();
    let (mut weight, mut accumulated, mut best, mut cutoff) = (0u64, 0., -1., 128);
    for (i, &v) in hist.iter().enumerate() {
        weight += v;
        if weight == 0 {
            continue;
        }
        if weight == total {
            break;
        }
        accumulated += i as f64 * v as f64;
        let delta = accumulated / weight as f64 - (sum - accumulated) / (total - weight) as f64;
        let between = weight as f64 * (total - weight) as f64 * delta * delta;
        if between > best {
            best = between;
            cutoff = i as u8;
        }
    }
    cutoff
}
pub(crate) fn mask(
    image: &RgbaImage,
    o: &TraceOptions,
    control: &WorkControl,
) -> Result<Vec<u16>, String> {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let cutoff = if o.mono_threshold == "fixed" {
        o.mono_cutoff
    } else {
        otsu(image)
    } as f64;
    if o.mono_threshold != "adaptive" {
        return image
            .as_raw()
            .chunks(4)
            .enumerate()
            .map(|(i, p)| {
                if i & 8191 == 0 {
                    control.check()?;
                }
                Ok(
                    if p[3] >= 128 && ((luma(p).round() <= cutoff) != o.mono_invert) {
                        0
                    } else {
                        TRANSPARENT
                    },
                )
            })
            .collect();
    }
    let stride = w + 1;
    let _scratch = control.reserve(stride * (h + 1) * 16)?;
    let mut sums = vec![0.; stride * (h + 1)];
    let mut counts = vec![0u64; stride * (h + 1)];
    for y in 0..h {
        control.check()?;
        let (mut sum, mut count) = (0., 0);
        for x in 0..w {
            let p = image.get_pixel(x as u32, y as u32);
            if p[3] >= 128 {
                sum += luma(&p.0);
                count += 1;
            }
            let i = (y + 1) * stride + x + 1;
            sums[i] = sum + sums[i - stride];
            counts[i] = count + counts[i - stride];
        }
    }
    let side = if o.mono_adaptive_window == 0 {
        (w.min(h) as f64 / 8.).round().max(3.) as usize
    } else {
        o.mono_adaptive_window as usize
    };
    let half = (side / 2).max(1);
    let keep = (100 - o.mono_adaptive_percent) as f64 / 100.;
    let mut result = vec![TRANSPARENT; w * h];
    for y in 0..h {
        control.check()?;
        for x in 0..w {
            let p = image.get_pixel(x as u32, y as u32);
            if p[3] < 128 {
                continue;
            }
            let (x0, y0, x1, y1) = (
                x.saturating_sub(half),
                y.saturating_sub(half),
                (x + half + 1).min(w),
                (y + half + 1).min(h),
            );
            let (a, b, c, d) = (
                y0 * stride + x0,
                y0 * stride + x1,
                y1 * stride + x0,
                y1 * stride + x1,
            );
            let count = counts[d] + counts[a] - counts[b] - counts[c];
            if count > 0
                && ((luma(&p.0) <= (sums[d] + sums[a] - sums[b] - sums[c]) / count as f64 * keep)
                    != o.mono_invert)
            {
                result[y * w + x] = 0;
            }
        }
    }
    Ok(result)
}
