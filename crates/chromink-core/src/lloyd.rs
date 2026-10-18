//! Oklab Wu initialization followed by full weighted Lloyd iterations.
use crate::{control::WorkControl, TraceOptions};
use palette::{Oklab, Srgb};
use quantette::{Pipeline, QuantizeMethod};
use rayon::prelude::*;
use std::collections::HashMap;
fn lloyd(
    rgb: &[Srgb<u8>],
    initial: &[Oklab],
    rounds: usize,
    control: &WorkControl,
) -> Result<Vec<Oklab>, String> {
    // Exact RGB deduplication, unlike Vecline's approximate 5-bit bins.
    let mut counts = HashMap::<[u8; 3], u64>::new();
    for (i, p) in rgb.iter().enumerate() {
        if i & 8191 == 0 {
            control.check()?;
        }
        *counts.entry([p.red, p.green, p.blue]).or_default() += 1;
    }
    let mut keys: Vec<_> = counts.into_iter().collect();
    keys.sort_unstable_by_key(|p| p.0);
    let source: Vec<_> = keys
        .iter()
        .map(|(p, _)| Srgb::new(p[0], p[1], p[2]))
        .collect();
    let labs = quantette::color_space::srgb8_to_oklab(&source);
    let points: Vec<_> = labs
        .iter()
        .zip(&keys)
        .map(|(p, (_, w))| ([p.l as f64, p.a as f64, p.b as f64], *w as f64))
        .collect();
    let mut centers: Vec<_> = initial
        .iter()
        .map(|p| [p.l as f64, p.a as f64, p.b as f64])
        .collect();
    for _ in 0..rounds {
        control.check()?;
        let partial: Result<Vec<_>, String> = points
            .par_chunks(8192)
            .map(|part| {
                control.check()?;
                let mut sums = vec![[0f64; 4]; centers.len()];
                for (p, w) in part {
                    let best = (0..centers.len())
                        .min_by(|&a, &b| {
                            let dist = |c: usize| {
                                (0..3).map(|j| (p[j] - centers[c][j]).powi(2)).sum::<f64>()
                            };
                            dist(a).total_cmp(&dist(b)).then(a.cmp(&b))
                        })
                        .unwrap();
                    for j in 0..3 {
                        sums[best][j] += p[j] * w;
                    }
                    sums[best][3] += w;
                }
                Ok(sums)
            })
            .collect();
        let mut sums = vec![[0f64; 4]; centers.len()];
        for part in partial? {
            for (a, b) in sums.iter_mut().zip(part) {
                for j in 0..4 {
                    a[j] += b[j];
                }
            }
        }
        let mut moved = false;
        for (c, s) in centers.iter_mut().zip(sums) {
            if s[3] > 0. {
                let next = std::array::from_fn(|j| s[j] / s[3]);
                moved |= *c != next;
                *c = next;
            }
        }
        if !moved {
            break;
        }
    }
    Ok(centers
        .into_iter()
        .map(|p| Oklab::new(p[0] as f32, p[1] as f32, p[2] as f32))
        .collect())
}
pub(crate) fn palette(
    input: &[[u8; 4]],
    options: &TraceOptions,
    control: &WorkControl,
) -> Result<Vec<[u8; 3]>, String> {
    control.check()?;
    let _memory = control.reserve(input.len().saturating_mul(96))?;
    let rgb: Vec<_> = input
        .iter()
        .filter(|p| p[3] > 0)
        .map(|p| Srgb::new(p[0], p[1], p[2]))
        .collect();
    if rgb.is_empty() {
        return Ok(Vec::new());
    }
    let initial = Pipeline::new()
        .palette_size(options.colors.try_into().map_err(|e| format!("{e}"))?)
        .quantize_method(QuantizeMethod::Wu)
        .parallel(rayon::current_num_threads() > 1)
        .input_slice(&rgb)
        .map_err(|e| e.to_string())?
        .output_oklab_palette();
    control.check()?;
    let refined = if options.lloyd_iterations == 0 {
        initial.iter().copied().collect()
    } else {
        lloyd(&rgb, &initial, options.lloyd_iterations as usize, control)?
    };
    Ok(quantette::color_space::oklab_to_srgb8(&refined)
        .into_iter()
        .map(|p| [p.red, p.green, p.blue])
        .collect())
}
