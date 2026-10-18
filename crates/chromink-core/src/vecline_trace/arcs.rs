//! Fit each shared crack boundary once, pin junctions, reuse its exact reverse.
use super::{
    fit::{self, P, S},
    subpixel, Options,
};
use crate::control::WorkControl;
use std::collections::HashMap;
fn dense(p: &[P]) -> Vec<P> {
    let mut out = Vec::new();
    for i in 0..p.len() {
        let a = p[i];
        let b = p[(i + 1) % p.len()];
        let n = ((b.x - a.x).abs() + (b.y - a.y).abs()) as usize;
        let dx = if b.x == a.x { 0. } else { (b.x - a.x).signum() };
        let dy = if b.y == a.y { 0. } else { (b.y - a.y).signum() };
        for k in 0..n {
            out.push(P::new(a.x + dx * k as f64, a.y + dy * k as f64));
        }
    }
    out
}
fn at(classes: &[i32], w: usize, h: usize, x: i32, y: i32) -> i32 {
    if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
        -2
    } else {
        classes[y as usize * w + x as usize]
    }
}
fn degree(classes: &[i32], w: usize, h: usize, p: P) -> usize {
    let (x, y) = (p.x as i32, p.y as i32);
    let v = [
        at(classes, w, h, x - 1, y - 1),
        at(classes, w, h, x, y - 1),
        at(classes, w, h, x, y),
        at(classes, w, h, x - 1, y),
    ];
    (0..4).filter(|&i| v[i] != v[(i + 1) % 4]).count()
}
fn neighbor(classes: &[i32], w: usize, h: usize, a: P, b: P, cls: i32) -> i32 {
    let (a, b) = if a.y == b.y {
        (
            (a.x.min(b.x) as i32, a.y as i32 - 1),
            (a.x.min(b.x) as i32, a.y as i32),
        )
    } else {
        (
            (a.x as i32 - 1, a.y.min(b.y) as i32),
            (a.x as i32, a.y.min(b.y) as i32),
        )
    };
    let v = at(classes, w, h, a.0, a.1);
    if v != cls {
        v
    } else {
        at(classes, w, h, b.0, b.1)
    }
}
fn key(p: &[P]) -> String {
    p.iter()
        .map(|p| format!("{},{}", p.x as i32, p.y as i32))
        .collect::<Vec<_>>()
        .join(";")
}
pub(super) fn fit(
    loops: &[Vec<Vec<P>>],
    classes: &[i32],
    img: &image::RgbaImage,
    o: &Options,
    c: &WorkControl,
) -> Result<Vec<Vec<Vec<S>>>, String> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let vertices: usize = loops
        .iter()
        .flatten()
        .map(|p| {
            (0..p.len())
                .map(|i| {
                    ((p[(i + 1) % p.len()].x - p[i].x).abs()
                        + (p[(i + 1) % p.len()].y - p[i].y).abs()) as usize
                })
                .sum::<usize>()
        })
        .sum();
    let _scratch = c.reserve(vertices.saturating_mul(240))?;
    let mut cache: HashMap<String, (P, Vec<S>)> = HashMap::new();
    let mut out = Vec::with_capacity(loops.len());
    for (cls, loops) in loops.iter().enumerate() {
        let mut faces = Vec::new();
        for loop_ in loops {
            c.check()?;
            let p = pinned_points(loop_, classes, w, h);
            let n = p.len();
            if n < 3 {
                continue;
            }
            let sides: Vec<_> = (0..n)
                .map(|i| neighbor(classes, w, h, p[i], p[(i + 1) % n], cls as i32))
                .collect();
            let cuts: Vec<_> = (0..n)
                .filter(|&i| sides[(i + n - 1) % n] != sides[i] || degree(classes, w, h, p[i]) != 2)
                .collect();
            let mut segments = Vec::new();
            if cuts.is_empty() {
                let start = (0..n)
                    .min_by(|&a, &b| {
                        p[a].x
                            .total_cmp(&p[b].x)
                            .then_with(|| p[a].y.total_cmp(&p[b].y))
                    })
                    .unwrap();
                let forward: Vec<_> = (0..n).map(|i| p[(start + i) % n]).collect();
                let reverse: Vec<_> = (0..n).map(|i| p[(start + n - i) % n]).collect();
                let f = key(&forward);
                let r = key(&reverse);
                let (key, base, reversed) = if f <= r {
                    (f, forward, false)
                } else {
                    (r, reverse, true)
                };
                let entry = if let Some(v) = cache.get(&key) {
                    v.clone()
                } else {
                    let mut options = o.clone();
                    options.vc_lattice_simplify = false;
                    options.vc_regularise = o.vc_regularise.max(2);
                    let fitted = fit::closed(&base, &options, c)?;
                    let start = fitted.last().map(|s| s.points[2]).unwrap_or(base[0]);
                    cache.insert(key, (start, fitted.clone()));
                    (start, fitted)
                };
                segments = if reversed {
                    fit::reverse(entry.0, &entry.1)
                } else {
                    entry.1
                };
            } else {
                for j in 0..cuts.len() {
                    let start = cuts[j];
                    let end = cuts[(j + 1) % cuts.len()];
                    let length = (end + n - start) % n;
                    let base: Vec<_> = (0..=length).map(|k| p[(start + k) % n]).collect();
                    let mut reverse = base.clone();
                    reverse.reverse();
                    let f = key(&base);
                    let r = key(&reverse);
                    let (key, base, reversed) = if f <= r {
                        (f, base, false)
                    } else {
                        (r, reverse, true)
                    };
                    let entry = if let Some(v) = cache.get(&key) {
                        v.clone()
                    } else {
                        let refined = if sides[start] >= 0 {
                            subpixel::open(&base, img, classes, cls as i32, c)?
                        } else {
                            base
                        };
                        let mut options = o.clone();
                        options.vc_regularise = o.vc_regularise.max(2);
                        if hard_junction(classes, w, h, base_first(&refined))
                            || hard_junction(classes, w, h, *refined.last().unwrap())
                        {
                            options.vc_optimize_error = o.vc_fit_error;
                        }
                        let fitted = fit::open(&refined, &options, c)?;
                        let start = refined[0];
                        cache.insert(key, (start, fitted.clone()));
                        (start, fitted)
                    };
                    let fitted = if reversed {
                        fit::reverse(entry.0, &entry.1)
                    } else {
                        entry.1
                    };
                    segments.extend(fitted);
                }
            }
            faces.push(segments);
        }
        out.push(faces);
    }
    Ok(out)
}

fn base_first(p: &[P]) -> P {
    p[0]
}
fn hard_junction(classes: &[i32], w: usize, h: usize, p: P) -> bool {
    let (x, y) = (p.x as i32, p.y as i32);
    let mut ids = [
        at(classes, w, h, x - 1, y - 1),
        at(classes, w, h, x, y - 1),
        at(classes, w, h, x, y),
        at(classes, w, h, x - 1, y),
    ];
    for id in &mut ids {
        if *id < 0 {
            *id = -1;
        }
    }
    ids.sort_unstable();
    1 + (1..4).filter(|&i| ids[i] != ids[i - 1]).count() >= 3
}
fn pinned_points(loop_: &[P], classes: &[i32], w: usize, h: usize) -> Vec<P> {
    let p = dense(loop_);
    let n = p.len();
    (0..n)
        .filter(|&i| {
            let a = p[(i + n - 1) % n];
            let b = p[i];
            let c = p[(i + 1) % n];
            (b - a).cross(c - b) != 0. || degree(classes, w, h, b) != 2
        })
        .map(|i| p[i])
        .collect()
}
pub(super) fn regularise(
    loops: &[Vec<Vec<P>>],
    classes: &[i32],
    w: usize,
    h: usize,
    o: &Options,
    c: &WorkControl,
) -> Result<Vec<Vec<Vec<P>>>, String> {
    loops
        .iter()
        .map(|loops| {
            loops
                .iter()
                .map(|loop_| {
                    c.check()?;
                    let original = pinned_points(loop_, classes, w, h);
                    let n = original.len();
                    let mut p = original.clone();
                    let _scratch = c.reserve(n * 64)?;
                    if n >= 4 && o.vc_regularise_band > 0. {
                        let pinned: Vec<_> = original
                            .iter()
                            .map(|&p| {
                                p.x <= 0.
                                    || p.y <= 0.
                                    || p.x >= w as f64
                                    || p.y >= h as f64
                                    || degree(classes, w, h, p) != 2
                            })
                            .collect();
                        for _ in 0..o.vc_regularise {
                            c.check()?;
                            let mut next = p.clone();
                            for i in 0..n {
                                if pinned[i] {
                                    continue;
                                }
                                let smooth =
                                    (p[i] + (p[(i + n - 1) % n] + p[(i + 1) % n]) * 0.5) * 0.5;
                                let d = smooth - original[i];
                                let len = d.x.hypot(d.y);
                                next[i] = original[i]
                                    + d * if len > o.vc_regularise_band {
                                        o.vc_regularise_band / len
                                    } else {
                                        1.
                                    };
                            }
                            p = next;
                        }
                    }
                    Ok(p)
                })
                .collect()
        })
        .collect()
}
