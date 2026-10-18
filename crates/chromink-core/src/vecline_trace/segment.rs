//! Felzenszwalb-Huttenlocher segmentation on the eight-connected Oklab/alpha grid.
use crate::{control::WorkControl, vecline};
use image::RgbaImage;
fn root(parent: &mut [usize], mut i: usize) -> usize {
    let mut r = i;
    while parent[r] != r {
        r = parent[r];
    }
    while parent[i] != r {
        let next = parent[i];
        parent[i] = r;
        i = next;
    }
    r
}
pub(super) fn flatten(
    input: &RgbaImage,
    k: f64,
    min_region: usize,
    c: &WorkControl,
) -> Result<RgbaImage, String> {
    if k <= 0. {
        return Ok(input.clone());
    }
    let (w, h) = (input.width() as usize, input.height() as usize);
    let n = w * h;
    let _scratch = c.reserve(n * 180)?;
    let lab: Vec<_> = input
        .pixels()
        .map(|p| {
            let v = vecline::lab([p[0], p[1], p[2]]);
            [v[0], v[1], v[2], p[3] as f64 / 255.]
        })
        .collect();
    let mut edges = Vec::with_capacity(n * 4);
    for y in 0..h {
        c.check()?;
        for x in 0..w {
            let a = y * w + x;
            for b in [
                if x + 1 < w { Some(a + 1) } else { None },
                if y + 1 < h { Some(a + w) } else { None },
                if x + 1 < w && y + 1 < h {
                    Some(a + w + 1)
                } else {
                    None
                },
                if x > 0 && y + 1 < h {
                    Some(a + w - 1)
                } else {
                    None
                },
            ]
            .into_iter()
            .flatten()
            {
                let d = (0..4)
                    .map(|j| (lab[a][j] - lab[b][j]).powi(2))
                    .sum::<f64>()
                    .sqrt();
                edges.push((a, b, d));
            }
        }
    }
    // Stable ties preserve the upstream scan order; no per-edge heap objects.
    edges.sort_by(|a, b| a.2.total_cmp(&b.2));
    c.check()?;
    let mut parent: Vec<_> = (0..n).collect();
    let mut size = vec![1usize; n];
    let mut internal = vec![0f64; n];
    for (i, &(a, b, d)) in edges.iter().enumerate() {
        if i & 8191 == 0 {
            c.check()?;
        }
        let a = root(&mut parent, a);
        let b = root(&mut parent, b);
        if a == b || d > (internal[a] + k / size[a] as f64).min(internal[b] + k / size[b] as f64) {
            continue;
        }
        let (big, small) = if size[a] >= size[b] { (a, b) } else { (b, a) };
        parent[small] = big;
        size[big] += size[small];
        internal[big] = d;
    }
    if min_region > 0 {
        // Keep insertion order for exact tie-breaking and sequential absorption.
        let mut borders: Vec<(usize, Vec<(usize, usize)>)> = Vec::new();
        let mut lookup = std::collections::HashMap::new();
        for &(a, b, _) in &edges {
            let a = root(&mut parent, a);
            let b = root(&mut parent, b);
            if a == b {
                continue;
            }
            for (r, other) in [(a, b), (b, a)] {
                if size[r] >= min_region {
                    continue;
                }
                let at = *lookup.entry(r).or_insert_with(|| {
                    borders.push((r, Vec::new()));
                    borders.len() - 1
                });
                let list = &mut borders[at].1;
                if let Some(p) = list.iter_mut().find(|p| p.0 == other) {
                    p.1 += 1;
                } else {
                    list.push((other, 1));
                }
            }
        }
        for (r, list) in borders {
            c.check()?;
            if root(&mut parent, r) != r || size[r] >= min_region {
                continue;
            }
            let total: usize = list.iter().map(|p| p.1).sum();
            let best = list
                .iter()
                .enumerate()
                .max_by(|(i, a), (j, b)| a.1.cmp(&b.1).then_with(|| j.cmp(i)))
                .map(|(_, p)| *p);
            if let Some((best, len)) = best {
                if len as f64 / total as f64 >= 0.67 {
                    let target = root(&mut parent, best);
                    if target != r {
                        parent[r] = target;
                        size[target] += size[r];
                    }
                }
            }
        }
    }
    let mut sums = vec![[0f64; 4]; n];
    let mut counts = vec![0usize; n];
    for i in 0..n {
        if i & 8191 == 0 {
            c.check()?;
        }
        let r = root(&mut parent, i);
        for j in 0..4 {
            sums[r][j] += lab[i][j];
        }
        counts[r] += 1;
    }
    let mut colors = vec![[0u8; 4]; n];
    for i in 0..n {
        if counts[i] == 0 {
            continue;
        }
        let v = sums[i].map(|v| v / counts[i] as f64);
        let rgb = super::math::srgb([v[0], v[1], v[2]]);
        colors[i] = [rgb[0], rgb[1], rgb[2], (v[3] * 255.).round() as u8];
    }
    let mut out = input.clone();
    for (i, p) in out.pixels_mut().enumerate() {
        *p = image::Rgba(colors[root(&mut parent, i)]);
    }
    Ok(out)
}
