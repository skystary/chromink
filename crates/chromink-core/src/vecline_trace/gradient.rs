//! Vecline gradient detection: adjacent palette bands, Oklab regression, linear/radial
//! 32-bin models, adaptive stops and validation against the original pixels.
use super::{math, Options};
use crate::{control::WorkControl, vecline, Gradient};
use image::RgbaImage;
fn root(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}
fn render(stops: &[(f64, [u8; 3])], u: f64) -> [u8; 3] {
    if u <= stops[0].0 {
        return stops[0].1;
    }
    for pair in stops.windows(2) {
        if u <= pair[1].0 {
            let f = (u - pair[0].0) / (pair[1].0 - pair[0].0);
            return std::array::from_fn(|j| {
                (pair[0].1[j] as f64 * (1. - f) + pair[1].1[j] as f64 * f).round() as u8
            });
        }
    }
    stops.last().unwrap().1
}
#[derive(Clone, Copy, Default)]
struct Bin {
    sum: [f64; 3],
    sq: f64,
    n: f64,
}
fn stops(bins: &[Bin; 32], max: usize, target: f64) -> (Vec<(f64, [u8; 3])>, f64) {
    let mut curve = [[0.; 3]; 32];
    let first = bins.iter().position(|v| v.n > 0.).unwrap_or(0);
    let mut last = bins[first].sum.map(|v| v / bins[first].n.max(1.));
    let mut dev = 0.;
    let mut count = 0.;
    for j in 0..32 {
        let b = bins[j];
        if b.n > 0. {
            last = b.sum.map(|v| v / b.n);
            dev += (b.sq - b.sum.iter().map(|v| v * v).sum::<f64>() / b.n).max(0.);
            count += b.n;
        }
        curve[j] = last;
    }
    let mut chosen = vec![0, 31];
    loop {
        let out: Vec<_> = chosen
            .iter()
            .map(|&j| (j as f64 / 31., math::srgb(curve[j])))
            .collect();
        let mut worst = None;
        let mut worst_error = -1.;
        let mut error = 0.;
        for j in 0..32 {
            if bins[j].n == 0. {
                continue;
            }
            let e =
                vecline::distance(vecline::lab(render(&out, j as f64 / 31.)), curve[j]) * bins[j].n;
            error += e;
            if e > worst_error && !chosen.contains(&j) {
                worst_error = e;
                worst = Some(j);
            }
        }
        if chosen.len() >= max || worst.is_none() || error / count.max(1.) <= target {
            return (out, (dev / count.max(1.)).sqrt());
        }
        chosen.push(worst.unwrap());
        chosen.sort_unstable();
    }
}
pub(super) fn detect(
    img: &RgbaImage,
    classes: &mut [i32],
    palette: &mut Vec<[u8; 4]>,
    o: &Options,
    c: &WorkControl,
) -> Result<Vec<Option<Gradient>>, String> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let n = w * h;
    let mut paints = vec![None; palette.len()];
    if !o.vc_gradients {
        return Ok(paints);
    }
    let _scratch = c.reserve(n * 80)?;
    let labs: Vec<_> = palette
        .iter()
        .map(|p| vecline::lab([p[0], p[1], p[2]]))
        .collect();
    let mut parent: Vec<_> = (0..n).collect();
    for y in 0..h {
        c.check()?;
        for x in 0..w {
            let i = y * w + x;
            let a = classes[i];
            if a < 0 {
                continue;
            }
            for j in [
                if x + 1 < w { Some(i + 1) } else { None },
                if y + 1 < h { Some(i + w) } else { None },
            ]
            .into_iter()
            .flatten()
            {
                let b = classes[j];
                if b < 0
                    || palette[a as usize][3] != palette[b as usize][3]
                    || vecline::distance(labs[a as usize], labs[b as usize])
                        > o.vc_gradient_step_max.powi(2)
                {
                    continue;
                }
                let a = root(&mut parent, i);
                let b = root(&mut parent, j);
                parent[a.max(b)] = a.min(b);
            }
        }
    }
    let mut groups: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for i in 0..n {
        if i & 8191 == 0 {
            c.check()?;
        }
        if classes[i] >= 0 {
            groups.entry(root(&mut parent, i)).or_default().push(i);
        }
    }
    let min_area = if o.vc_gradient_min_area == 0 {
        64.max((n as f64 / 2000.).round() as usize)
    } else {
        o.vc_gradient_min_area as usize
    };
    for indices in groups.values() {
        c.check()?;
        if indices.len() < min_area {
            continue;
        }
        let bands: std::collections::BTreeSet<_> = indices
            .iter()
            .map(|&i| {
                (
                    palette[classes[i] as usize][0],
                    palette[classes[i] as usize][1],
                    palette[classes[i] as usize][2],
                )
            })
            .collect();
        if bands.len() < 2 {
            continue;
        }
        let mut matrix = [[0f64; 3]; 3];
        let mut rhs = [[0f64; 3]; 3];
        let mut flat = 0.;
        let mut samples = Vec::with_capacity(indices.len());
        let mut mean = [0.; 2];
        for &i in indices {
            let x = (i % w) as f64;
            let y = (i / w) as f64;
            mean[0] += x;
            mean[1] += y;
            let p = img.get_pixel(x as u32, y as u32);
            let lab = vecline::lab([p[0], p[1], p[2]]);
            flat += vecline::distance(lab, labs[classes[i] as usize]);
            samples.push((x, y, lab));
            let v = [1., x, y];
            for a in 0..3 {
                for b in 0..3 {
                    matrix[a][b] += v[a] * v[b];
                    rhs[a][b] += lab[a] * v[b];
                }
            }
        }
        let count = indices.len() as f64;
        mean = mean.map(|v| v / count);
        let model: Option<Vec<_>> = rhs.iter().map(|&rhs| math::solve(matrix, rhs)).collect();
        let axis = model.and_then(|m| {
            let a = m.iter().map(|v| v[1] * v[1]).sum::<f64>();
            let b = m.iter().map(|v| v[1] * v[2]).sum::<f64>();
            let cc = m.iter().map(|v| v[2] * v[2]).sum::<f64>();
            if a + cc < 1e-12 {
                return None;
            }
            let lambda = (a + cc) / 2. + (((a - cc) / 2.).powi(2) + b * b).sqrt();
            let (x, y) = if b.abs() > 1e-12 {
                (lambda - cc, b)
            } else if a >= cc {
                (1., 0.)
            } else {
                (0., 1.)
            };
            let len = x.hypot(y);
            (len >= 1e-12).then_some([x / len, y / len])
        });
        let radius = samples
            .iter()
            .map(|&(x, y, _)| (x - mean[0]).hypot(y - mean[1]))
            .fold(0f64, f64::max);
        let mut best: Option<(f64, Gradient)> = None;
        for radial in [false, true] {
            let Some(axis) = (if radial { Some([1., 0.]) } else { axis }) else {
                continue;
            };
            let positions: Vec<_> = samples
                .iter()
                .map(|&(x, y, _)| {
                    if radial {
                        (x - mean[0]).hypot(y - mean[1])
                    } else {
                        axis[0] * x + axis[1] * y
                    }
                })
                .collect();
            let lo = if radial {
                0.
            } else {
                positions.iter().copied().fold(f64::INFINITY, f64::min)
            };
            let hi = if radial {
                radius
            } else {
                positions.iter().copied().fold(f64::NEG_INFINITY, f64::max)
            };
            if hi <= lo {
                continue;
            }
            let mut bins = [Bin::default(); 32];
            for (&position, &(_, _, lab)) in positions.iter().zip(&samples) {
                let b = (((position - lo) / (hi - lo)).clamp(0., 1.) * 32.)
                    .floor()
                    .min(31.) as usize;
                for j in 0..3 {
                    bins[b].sum[j] += lab[j];
                    bins[b].sq += lab[j] * lab[j];
                }
                bins[b].n += 1.;
            }
            let target = flat / count * (1. - o.vc_gradient_margin);
            let (stops, rms) = stops(&bins, o.vc_gradient_stops as usize, target);
            if rms > o.vc_gradient_max_error {
                continue;
            }
            let error = positions
                .iter()
                .zip(&samples)
                .map(|(&pos, &(_, _, lab))| {
                    vecline::distance(
                        lab,
                        vecline::lab(render(&stops, ((pos - lo) / (hi - lo)).clamp(0., 1.))),
                    )
                })
                .sum::<f64>()
                / count;
            if error >= target * bands.len() as f64 || error >= o.vc_gradient_max_error.powi(2) {
                continue;
            }
            let mean_t = axis[0] * mean[0] + axis[1] * mean[1];
            let g = if radial {
                Gradient {
                    radial,
                    from: mean,
                    to: [radius, 0.],
                    stops,
                }
            } else {
                Gradient {
                    radial,
                    from: [
                        mean[0] + axis[0] * (lo - mean_t),
                        mean[1] + axis[1] * (lo - mean_t),
                    ],
                    to: [
                        mean[0] + axis[0] * (hi - mean_t),
                        mean[1] + axis[1] * (hi - mean_t),
                    ],
                    stops,
                }
            };
            if best.as_ref().is_none_or(|v| error < v.0) {
                best = Some((error, g));
            }
        }
        if let Some((_, g)) = best {
            let id = palette.len();
            let color = palette[classes[indices[0]] as usize];
            palette.push(color);
            paints.push(Some(g));
            for &i in indices {
                classes[i] = id as i32;
            }
        }
    }
    Ok(paints)
}
