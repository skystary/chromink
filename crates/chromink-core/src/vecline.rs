//! Rust implementation of Vecline's RGB Wu + Oklab palette refinement.
//! Algorithm provenance and MIT notice: documentation/algorithms/VECLINE-LICENSE.txt.
//! Only palette construction is implemented here; Chromink owns outlines and dithering.
use crate::control::WorkControl;
use rayon::prelude::*;
use std::sync::OnceLock;
const SIDE: usize = 33;
const STRIDE: usize = SIDE * SIDE;
const VOLUME: usize = SIDE * SIDE * SIDE;
#[derive(Clone, Copy, Default)]
pub(crate) struct Bin {
    pub count: u64,
    pub sum: [u64; 3],
    square: u64,
}
pub(crate) fn histogram(input: &[[u8; 4]], control: &WorkControl) -> Result<Vec<Bin>, String> {
    // Fixed chunk boundaries and integer accumulation make worker count irrelevant.
    // Bound temporary histograms by the pool size, rather than the number of pixels.
    let chunk = input
        .len()
        .div_ceil(rayon::current_num_threads().min(8).max(1))
        .max(65536);
    let parts: Result<Vec<_>, String> = input
        .par_chunks(chunk)
        .map(|pixels| {
            let mut bins = vec![Bin::default(); VOLUME];
            for (i, p) in pixels.iter().enumerate() {
                if i & 8191 == 0 {
                    control.check()?;
                }
                if p[3] == 0 {
                    continue;
                }
                let index = ((p[0] as usize >> 3) + 1) * STRIDE
                    + ((p[1] as usize >> 3) + 1) * SIDE
                    + (p[2] as usize >> 3)
                    + 1;
                let bin = &mut bins[index];
                bin.count += 1;
                for c in 0..3 {
                    bin.sum[c] += p[c] as u64;
                    bin.square += (p[c] as u64).pow(2);
                }
            }
            Ok(bins)
        })
        .collect();
    let mut parts = parts?.into_iter();
    let mut result = parts.next().unwrap_or_else(|| vec![Bin::default(); VOLUME]);
    for part in parts {
        control.check()?;
        for (a, b) in result.iter_mut().zip(part) {
            a.count += b.count;
            a.square += b.square;
            for c in 0..3 {
                a.sum[c] += b.sum[c];
            }
        }
    }
    Ok(result)
}
pub(crate) fn rgb(bin: &Bin) -> [u8; 3] {
    std::array::from_fn(|c| ((bin.sum[c] as f64 / bin.count as f64).round()).clamp(0., 255.) as u8)
}
pub(crate) fn lab(rgb: [u8; 3]) -> [f64; 3] {
    static LINEAR: OnceLock<[f64; 256]> = OnceLock::new();
    let table = LINEAR.get_or_init(|| {
        std::array::from_fn(|i| {
            let v = i as f64 / 255.;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        })
    });
    let [r, g, b] = rgb.map(|x| table[x as usize]);
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
    ]
}
fn to_rgb([l, a, b]: [f64; 3]) -> [u8; 3] {
    let x = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let y = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let z = (l - 0.0894841775 * a - 1.291485548 * b).powi(3);
    [
        4.0767416621 * x - 3.3077115913 * y + 0.2309699292 * z,
        -1.2684380046 * x + 2.6097574011 * y - 0.3413193965 * z,
        -0.0041960863 * x - 0.7034186147 * y + 1.707614701 * z,
    ]
    .map(|v| {
        let v = if v <= 0.0031308 {
            12.92 * v
        } else {
            1.055 * v.powf(1. / 2.4) - 0.055
        };
        (v * 255.).round().clamp(0., 255.) as u8
    })
}
#[inline]
pub(crate) fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}
#[derive(Clone, Copy)]
struct Cube {
    low: [usize; 3],
    high: [usize; 3],
}
fn volume(c: Cube, m: &[[f64; 5]]) -> [f64; 5] {
    let mut v = [0.; 5];
    for mask in 0u8..8 {
        let p: [usize; 3] = std::array::from_fn(|i| {
            if mask & (1 << i) != 0 {
                c.high[i]
            } else {
                c.low[i]
            }
        });
        let sign = if mask.count_ones() % 2 == 1 { 1. } else { -1. };
        let cell = m[p[0] * STRIDE + p[1] * SIDE + p[2]];
        for j in 0..5 {
            v[j] += sign * cell[j];
        }
    }
    v
}
fn variance(c: Cube, m: &[[f64; 5]]) -> f64 {
    let v = volume(c, m);
    if v[0] == 0. {
        0.
    } else {
        v[4] - (v[1].powi(2) + v[2].powi(2) + v[3].powi(2)) / v[0]
    }
}
fn cut(c: Cube, m: &[[f64; 5]]) -> Option<(Cube, Cube)> {
    let total = volume(c, m);
    let mut best = -1.;
    let mut winner = None;
    for axis in 0..3 {
        for pos in c.low[axis] + 1..c.high[axis] {
            let mut left = c;
            left.high[axis] = pos;
            let a = volume(left, m);
            let b: [f64; 5] = std::array::from_fn(|j| total[j] - a[j]);
            if a[0] == 0. || b[0] == 0. {
                continue;
            }
            let score = (a[1].powi(2) + a[2].powi(2) + a[3].powi(2)) / a[0]
                + (b[1].powi(2) + b[2].powi(2) + b[3].powi(2)) / b[0];
            if score > best {
                best = score;
                let mut right = c;
                right.low[axis] = pos;
                winner = Some((left, right));
            }
        }
    }
    winner
}
fn bins_in<'a>(c: Cube, bins: &'a [Bin]) -> impl Iterator<Item = &'a Bin> {
    (c.low[0] + 1..=c.high[0])
        .flat_map(move |r| {
            (c.low[1] + 1..=c.high[1]).flat_map(move |g| {
                (c.low[2] + 1..=c.high[2]).map(move |b| &bins[r * STRIDE + g * SIDE + b])
            })
        })
        .filter(|b| b.count > 0)
}
pub(crate) fn palette(
    input: &[[u8; 4]],
    count: usize,
    iterations: u8,
    strategy: &str,
    control: &WorkControl,
) -> Result<Vec<[u8; 3]>, String> {
    let bins = histogram(input, control)?;
    let mut moments: Vec<[f64; 5]> = bins
        .iter()
        .map(|b| {
            [
                b.count as f64,
                b.sum[0] as f64,
                b.sum[1] as f64,
                b.sum[2] as f64,
                b.square as f64,
            ]
        })
        .collect();
    // Three separable prefix passes replace repeated box scans.
    for stride in [STRIDE, SIDE, 1] {
        control.check()?;
        for i in 0..VOLUME {
            if (i / stride) % SIDE > 0 {
                for c in 0..5 {
                    moments[i][c] += moments[i - stride][c];
                }
            }
        }
    }
    let mut cubes = vec![Cube {
        low: [0; 3],
        high: [32; 3],
    }];
    let mut errors = vec![0.];
    let mut next = 0;
    while cubes.len() < count {
        control.check()?;
        if let Some((a, b)) = cut(cubes[next], &moments) {
            cubes[next] = a;
            errors[next] = variance(a, &moments);
            cubes.push(b);
            errors.push(variance(b, &moments));
        } else {
            errors[next] = 0.;
        }
        next = (0..errors.len())
            .max_by(|&a, &b| errors[a].total_cmp(&errors[b]).then_with(|| b.cmp(&a)))
            .unwrap();
        if errors[next] <= 0. {
            break;
        }
    }
    let mut palette = Vec::new();
    for c in cubes {
        let v = volume(c, &moments);
        if v[0] == 0. {
            continue;
        }
        let value = if strategy == "dominant" {
            let mut best = None;
            for b in bins_in(c, &bins) {
                if best.is_none_or(|p: &Bin| b.count > p.count) {
                    best = Some(b);
                }
            }
            rgb(best.unwrap())
        } else if strategy == "median" {
            let entries: Vec<_> = bins_in(c, &bins).collect();
            std::array::from_fn(|channel| {
                let mut values: Vec<_> = entries
                    .iter()
                    .map(|b| (b.sum[channel] as f64 / b.count as f64, b.count))
                    .collect();
                values.sort_by(|a, b| a.0.total_cmp(&b.0));
                let total: u64 = values.iter().map(|p| p.1).sum();
                let mut cumulative = 0;
                for (value, n) in values {
                    cumulative += n;
                    if cumulative * 2 >= total {
                        return value.round().clamp(0., 255.) as u8;
                    }
                }
                0
            })
        } else {
            std::array::from_fn(|j| (v[j + 1] / v[0]).round().clamp(0., 255.) as u8)
        };
        palette.push(value);
    }
    if strategy != "mean" || iterations == 0 {
        return Ok(palette);
    }
    let points: Vec<_> = bins
        .iter()
        .filter(|b| b.count > 0)
        .map(|b| (lab(rgb(b)), b.count as f64))
        .collect();
    if points.len() <= palette.len() {
        return Ok(palette);
    }
    let mut centers: Vec<_> = palette.iter().map(|&p| lab(p)).collect();
    for _ in 0..iterations {
        control.check()?;
        let parts: Result<Vec<_>, String> = points
            .par_chunks(2048)
            .map(|part| {
                control.check()?;
                let mut sums = vec![[0.; 4]; centers.len()];
                for &(p, w) in part {
                    let best = (0..centers.len())
                        .min_by(|&a, &b| {
                            distance(p, centers[a])
                                .total_cmp(&distance(p, centers[b]))
                                .then(a.cmp(&b))
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
        let mut sums = vec![[0.; 4]; centers.len()];
        for part in parts? {
            for (s, p) in sums.iter_mut().zip(part) {
                for j in 0..4 {
                    s[j] += p[j];
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
    for (p, c) in palette.iter_mut().zip(centers) {
        *p = to_rgb(c);
    }
    Ok(palette)
}
