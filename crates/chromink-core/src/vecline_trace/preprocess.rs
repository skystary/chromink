//! Vecline selectiveBlur: separable Gaussian, clamped edges, four-channel gate.
use crate::control::WorkControl;
use image::RgbaImage;
pub(super) fn noise(input: &RgbaImage) -> f64 {
    let (w, h) = input.dimensions();
    if w < 3 || h < 3 {
        return 0.;
    }
    let lum = |x: u32, y: u32| {
        let p = input.get_pixel(x, y);
        (p[0] as f64 + p[1] as f64 + p[2] as f64) / 3. * p[3] as f64 / 255.
    };
    let mut total = 0.;
    let mut count = 0;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            if (lum(x + 1, y) - lum(x - 1, y)).hypot(lum(x, y + 1) - lum(x, y - 1)) > 24. {
                continue;
            }
            let mut mean = 0.;
            for ny in y - 1..=y + 1 {
                for nx in x - 1..=x + 1 {
                    mean += lum(nx, ny);
                }
            }
            total += (lum(x, y) - mean / 9.).abs();
            count += 1;
        }
    }
    if count == 0 {
        0.
    } else {
        total / count as f64
    }
}
pub(super) fn smooth(
    input: &RgbaImage,
    strength: f64,
    c: &WorkControl,
) -> Result<RgbaImage, String> {
    if strength <= 0. {
        return Ok(input.clone());
    }
    let iterations =
        (24. * strength * (1. - (-(noise(input) - 0.3).max(0.) / 0.5).exp())).round() as usize;
    if iterations == 0 {
        return Ok(input.clone());
    }
    let (w, h) = (input.width() as usize, input.height() as usize);
    let _scratch = c.reserve(input.len() * 16 + input.len())?;
    let mut cur: Vec<f64> = input.as_raw().iter().map(|&v| v as f64).collect();
    let mut next = cur.clone();
    for _ in 0..iterations {
        c.check()?;
        next.copy_from_slice(&cur);
        for y in 1..h - 1 {
            c.check()?;
            for x in 1..w - 1 {
                let p = (y * w + x) * 4;
                for ch in 0..3 {
                    let mut flow = 0.;
                    for q in [p - w * 4, p + w * 4, p - 4, p + 4] {
                        let d = cur[q + ch] - cur[p + ch];
                        flow += (-(d / 20.).powi(2)).exp() * d;
                    }
                    next[p + ch] = cur[p + ch] + 0.2 * flow;
                }
            }
        }
        std::mem::swap(&mut cur, &mut next);
    }
    Ok(RgbaImage::from_raw(
        w as u32,
        h as u32,
        cur.into_iter()
            .map(|v| v.round_ties_even().clamp(0., 255.) as u8)
            .collect(),
    )
    .unwrap())
}
pub(super) fn selective_blur(
    input: &RgbaImage,
    radius: u8,
    delta: u8,
    c: &WorkControl,
) -> Result<RgbaImage, String> {
    if radius == 0 {
        return Ok(input.clone());
    }
    let r = radius.min(5) as usize;
    let (w, h) = (input.width() as usize, input.height() as usize);
    let span = 2 * r + 1;
    let sigma = r as f64 / 3.;
    let mut kernel: Vec<_> = (0..span)
        .map(|i| {
            let x = i as f64 - r as f64;
            (-x * x / (2. * sigma * sigma)).exp()
        })
        .collect();
    let sum: f64 = kernel.iter().sum();
    for k in &mut kernel {
        *k /= sum;
    }
    let _scratch = c.reserve(span * w * 4 * 8 + w * h * 4)?;
    let mut ring = vec![0f64; span * w * 4];
    let mut ready = vec![usize::MAX; span];
    let mut out = input.clone();
    for y in 0..h {
        c.check()?;
        for offset in 0..span {
            let sy = (y as isize + offset as isize - r as isize).clamp(0, h as isize - 1) as usize;
            let slot = sy % span;
            if ready[slot] == sy {
                continue;
            }
            for x in 0..w {
                let mut values = [0.; 4];
                for (i, &weight) in kernel.iter().enumerate() {
                    let sx =
                        (x as isize + i as isize - r as isize).clamp(0, w as isize - 1) as usize;
                    let p = input.get_pixel(sx as u32, sy as u32);
                    for ch in 0..4 {
                        values[ch] += p[ch] as f64 * weight;
                    }
                }
                ring[slot * w * 4 + x * 4..slot * w * 4 + x * 4 + 4].copy_from_slice(&values);
            }
            ready[slot] = sy;
        }
        for x in 0..w {
            let mut values = [0.; 4];
            for (offset, &weight) in kernel.iter().enumerate() {
                let sy =
                    (y as isize + offset as isize - r as isize).clamp(0, h as isize - 1) as usize;
                for ch in 0..4 {
                    values[ch] += ring[(sy % span) * w * 4 + x * 4 + ch] * weight;
                }
            }
            let p = input.get_pixel(x as u32, y as u32);
            if values
                .iter()
                .enumerate()
                .all(|(ch, &v)| (v - p[ch] as f64).abs() <= delta as f64)
            {
                out.put_pixel(
                    x as u32,
                    y as u32,
                    image::Rgba(values.map(|v| v.round().clamp(0., 255.) as u8)),
                );
            }
        }
    }
    Ok(out)
}
