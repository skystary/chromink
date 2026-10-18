//! 减色与误差扩散。pngquant 的 Rust 量化核心保留为本地源码库。
use crate::{control::WorkControl, TraceOptions, TRANSPARENT};
use image::RgbaImage;
use rayon::prelude::*;
type Palette = Vec<[u8; 4]>;
pub fn reduce_controlled(
    image: &RgbaImage,
    options: &TraceOptions,
    control: &WorkControl,
) -> Result<(Palette, Vec<u16>), String> {
    control.check()?;
    let (input, remainder) = image.as_raw().as_chunks::<4>();
    debug_assert!(remainder.is_empty());
    if options.colors == 1 {
        if options.quantization == "vc" {
            return Ok((
                vec![[0, 0, 0, 255]],
                crate::threshold::mask(image, options, control)?,
            ));
        }
        // A one-ink output needs two trained classes (ink/paper), not one RGB
        // centroid. Keep the chosen reducer and all of its training controls.
        let mut two = options.clone();
        two.colors = 2;
        let (palette, indices) = reduce_controlled(image, &two, control)?;
        let ink: Vec<_> = palette
            .iter()
            .map(|p| u16::from(p[0]) + u16::from(p[1]) + u16::from(p[2]) < 384)
            .collect();
        let result = indices
            .into_iter()
            .map(|id| {
                if id != TRANSPARENT && ink[id as usize] {
                    0
                } else {
                    TRANSPARENT
                }
            })
            .collect();
        return Ok((vec![[0, 0, 0, 255]], result));
    }
    let perceptual = options.quantization == "wl" || options.quantization == "vc";
    let mut palette: Palette = {
        match options.quantization.as_str() {
            "wl" => crate::lloyd::palette(input, options, control)?
                .into_iter()
                .map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            "vc" => crate::vecline::palette(
                input,
                options.colors as usize,
                options.vecline_iterations,
                &options.vecline_strategy,
                control,
            )?
            .into_iter()
            .map(|p| [p[0], p[1], p[2], 255])
            .collect(),
            _ => {
                let rgba: &[imagequant::RGBA] = bytemuck::cast_slice(input);
                let mut attr = imagequant::new();
                attr.set_max_colors(options.colors as u32)
                    .map_err(|e| e.to_string())?;
                let cancellation = control.clone();
                attr.set_progress_callback(move |_| {
                    if cancellation.cancelled() {
                        imagequant::ControlFlow::Break
                    } else {
                        imagequant::ControlFlow::Continue
                    }
                });
                let mut source = attr
                    .new_image_borrowed(rgba, image.width() as usize, image.height() as usize, 0.)
                    .map_err(|e| e.to_string())?;
                let mut result = attr
                    .quantize(&mut source)
                    .map_err(|e| control.check().err().unwrap_or_else(|| e.to_string()))?;
                result
                    .palette()
                    .iter()
                    .map(|p| [p.r, p.g, p.b, p.a])
                    .collect()
            }
        }
    };
    palette.retain(|p| p[3] != 0);
    palette.sort_unstable();
    palette.dedup();
    if palette.is_empty() {
        return Ok((palette, vec![TRANSPARENT; input.len()]));
    }
    if perceptual && options.tracing != "vecline" {
        crate::perceptual::fit_alpha(&mut palette, input, control)?;
    }
    let pixels = remap_with_metric(
        &input,
        image.width() as usize,
        image.height() as usize,
        &palette,
        &options.dither,
        perceptual,
        control,
    )?;
    control.check()?;
    Ok((palette, pixels))
}
fn nearest(pixel: [f64; 4], palette: &[[u8; 4]]) -> usize {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if std::is_x86_feature_detected!("avx2") {
        // Runtime detection also checks the OS support for saving AVX state.
        return unsafe { nearest_avx2(pixel, palette) };
    }
    nearest_scalar(pixel, palette)
}
fn nearest_scalar(pixel: [f64; 4], palette: &[[u8; 4]]) -> usize {
    palette
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let error = |color: &[u8; 4]| {
                (0..4)
                    .map(|i| {
                        let d = pixel[i] - color[i] as f64;
                        d * d
                    })
                    .sum::<f64>()
            };
            error(a).total_cmp(&error(b))
        })
        .unwrap()
        .0
}
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn nearest_avx2(pixel: [f64; 4], palette: &[[u8; 4]]) -> usize {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::*;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::*;
    let input = unsafe { _mm256_loadu_pd(pixel.as_ptr()) };
    palette
        .iter()
        .enumerate()
        .map(|(index, color)| {
            let packed = _mm_cvtsi32_si128(u32::from_le_bytes(*color) as i32);
            let channels = _mm256_cvtepi32_pd(_mm_cvtepu8_epi32(packed));
            let delta = _mm256_sub_pd(input, channels);
            let squared = _mm256_mul_pd(delta, delta);
            let mut scores = [0.; 4];
            unsafe {
                _mm256_storeu_pd(scores.as_mut_ptr(), squared);
            }
            (index, scores.into_iter().sum::<f64>())
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap()
        .0
}
fn remap_with_metric(
    input: &[[u8; 4]],
    w: usize,
    h: usize,
    palette: &[[u8; 4]],
    dither: &str,
    perceptual: bool,
    control: &WorkControl,
) -> Result<Vec<u16>, String> {
    control.check()?;
    #[cfg(feature = "gpu")]
    if dither == "none" && !perceptual {
        if let Some(mapped) = crate::gpu::try_remap_controlled(input, palette, control) {
            control.check()?;
            return Ok(mapped);
        }
    }
    if dither != "floydsteinberg" && dither != "riemersma" {
        return if perceptual {
            crate::perceptual::Matcher::new(palette).map(input, control)
        } else {
            remap_cpu_no_dither_checked(input, palette, control)
        };
    }
    let matcher = perceptual.then(|| crate::perceptual::Matcher::new(palette));
    let choose = |color| {
        matcher
            .as_ref()
            .map_or_else(|| nearest(color, palette), |m| m.nearest(color))
    };
    let mut result = vec![TRANSPARENT; input.len()];
    if dither == "floydsteinberg" {
        let mut row = vec![[0.; 4]; w + 2];
        let mut next = vec![[0.; 4]; w + 2];
        for y in 0..h {
            control.check()?;
            for x in 0..w {
                let i = y * w + x;
                if input[i][3] == 0 {
                    continue;
                }
                let color =
                    std::array::from_fn(|k| (input[i][k] as f64 + row[x + 1][k]).clamp(0., 255.));
                let index = choose(color);
                result[i] = index as u16;
                for k in 0..4 {
                    let e = color[k] - palette[index][k] as f64;
                    row[x + 2][k] += e * 7. / 16.;
                    next[x][k] += e * 3. / 16.;
                    next[x + 1][k] += e * 5. / 16.;
                    next[x + 2][k] += e / 16.;
                }
            }
            std::mem::swap(&mut row, &mut next);
            next.fill([0.; 4]);
        }
    } else if dither == "riemersma" {
        let side = w.max(h).next_power_of_two();
        let mut queue = [[0.; 4]; 16];
        let mut head = 0;
        let weights: Vec<f64> = (0..16)
            .map(|i| (i as f64 / 15. * 16f64.ln()).exp())
            .collect();
        let total: f64 = weights.iter().sum();

        visit_hilbert_checked(
            w,
            h,
            0,
            0,
            0,
            side as i64,
            side as i64,
            0,
            control,
            |x, y| {
                let i = y * w + x;
                if input[i][3] == 0 {
                    return;
                }
                let color = std::array::from_fn(|k| {
                    (input[i][k] as f64
                        + (0..16)
                            .map(|j| queue[(head + j) % 16][k] * weights[j] / total)
                            .sum::<f64>())
                    .clamp(0., 255.)
                });
                let index = choose(color);
                result[i] = index as u16;
                queue[head] = std::array::from_fn(|k| color[k] - palette[index][k] as f64);
                head = (head + 1) % 16;
            },
        );
    } else {
        return if let Some(matcher) = matcher {
            matcher.map(input, control)
        } else {
            remap_cpu_no_dither_checked(input, palette, control)
        };
    }
    control.check()?;
    Ok(result)
}
#[cfg(all(test, feature = "gpu"))]
fn remap_cpu_no_dither(input: &[[u8; 4]], palette: &[[u8; 4]]) -> Vec<u16> {
    remap_cpu_no_dither_checked(input, palette, &WorkControl::unlimited()).unwrap()
}
fn remap_cpu_no_dither_checked(
    input: &[[u8; 4]],
    palette: &[[u8; 4]],
    control: &WorkControl,
) -> Result<Vec<u16>, String> {
    let matcher = crate::matching::Matcher::new(palette);
    let mut result = vec![TRANSPARENT; input.len()];
    result
        .par_chunks_mut(4096)
        .zip(input.par_chunks(4096))
        .try_for_each(|(output, pixels)| -> Result<(), String> {
            control.check()?;
            // High-entropy blocks do not repay a HashMap lookup for every pixel.
            let sample: std::collections::HashSet<_> = pixels
                .iter()
                .step_by((pixels.len() / 64).max(1))
                .take(64)
                .copied()
                .collect();
            let use_cache = sample.len() <= 48;
            let mut cache = std::collections::HashMap::new();
            for (index, p) in pixels.iter().enumerate() {
                if p[3] != 0 {
                    output[index] = if use_cache {
                        *cache.entry(*p).or_insert_with(|| matcher.nearest(*p))
                    } else {
                        matcher.nearest(*p)
                    };
                }
            }
            Ok(())
        })?;
    control.check()?;
    Ok(result)
}

// Skip whole out-of-image quadrants, so a long narrow image does not visit
// the billions of empty cells in its enclosing Hilbert square.

fn visit_hilbert_checked(
    w: usize,
    h: usize,
    x: i64,
    y: i64,
    xi: i64,
    xj: i64,
    yi: i64,
    yj: i64,
    control: &WorkControl,
    mut visit: impl FnMut(usize, usize),
) {
    fn walk(
        w: i64,
        h: i64,
        x: i64,
        y: i64,
        xi: i64,
        xj: i64,
        yi: i64,
        yj: i64,
        visit: &mut impl FnMut(usize, usize),
        checkpoint: &mut (usize, bool),
        control: &WorkControl,
    ) {
        checkpoint.0 += 1;
        if checkpoint.0 & 4095 == 0 {
            checkpoint.1 = control.cancelled();
        }
        if checkpoint.1 {
            return;
        }
        let xs = [x, x + xi, x + yi, x + xi + yi];
        let ys = [y, y + xj, y + yj, y + xj + yj];
        if *xs.iter().max().unwrap() <= 0
            || *xs.iter().min().unwrap() >= w
            || *ys.iter().max().unwrap() <= 0
            || *ys.iter().min().unwrap() >= h
        {
            return;
        }
        if xi.abs() + xj.abs() == 1 {
            let px = (x as f64 + (xi + yi) as f64 / 2.).floor() as i64;
            let py = (y as f64 + (xj + yj) as f64 / 2.).floor() as i64;
            if px >= 0 && py >= 0 && px < w && py < h {
                visit(px as usize, py as usize);
            }
        } else {
            walk(
                w,
                h,
                x,
                y,
                yi / 2,
                yj / 2,
                xi / 2,
                xj / 2,
                visit,
                checkpoint,
                control,
            );
            walk(
                w,
                h,
                x + xi / 2,
                y + xj / 2,
                xi / 2,
                xj / 2,
                yi / 2,
                yj / 2,
                visit,
                checkpoint,
                control,
            );
            walk(
                w,
                h,
                x + xi / 2 + yi / 2,
                y + xj / 2 + yj / 2,
                xi / 2,
                xj / 2,
                yi / 2,
                yj / 2,
                visit,
                checkpoint,
                control,
            );
            walk(
                w,
                h,
                x + xi / 2 + yi,
                y + xj / 2 + yj,
                -yi / 2,
                -yj / 2,
                -xi / 2,
                -xj / 2,
                visit,
                checkpoint,
                control,
            );
        }
    }
    walk(
        w as i64,
        h as i64,
        x,
        y,
        xi,
        xj,
        yi,
        yj,
        &mut visit,
        &mut (0, false),
        control,
    );
}
