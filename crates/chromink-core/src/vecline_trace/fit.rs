//! Rust port of Vecline fit.ts (MIT): DP/lattice simplification, Schneider
//! least squares, Newton refinement, continuous error checks and curve merges.
use super::Options;
use crate::control::WorkControl;
use std::ops::{Add, Mul, Neg, Sub};
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub(super) struct P {
    pub x: f64,
    pub y: f64,
}
impl P {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    fn dot(self, b: Self) -> f64 {
        self.x * b.x + self.y * b.y
    }
    pub fn cross(self, b: Self) -> f64 {
        self.x * b.y - self.y * b.x
    }
    fn len(self) -> f64 {
        self.x.hypot(self.y)
    }
    fn norm(self) -> Self {
        let l = self.len();
        if l == 0. {
            Self::new(0., 0.)
        } else {
            self * (1. / l)
        }
    }
}
impl Add for P {
    type Output = Self;
    fn add(self, b: Self) -> Self {
        Self::new(self.x + b.x, self.y + b.y)
    }
}
impl Sub for P {
    type Output = Self;
    fn sub(self, b: Self) -> Self {
        Self::new(self.x - b.x, self.y - b.y)
    }
}
impl Mul<f64> for P {
    type Output = Self;
    fn mul(self, b: f64) -> Self {
        Self::new(self.x * b, self.y * b)
    }
}
impl Neg for P {
    type Output = Self;
    fn neg(self) -> Self {
        self * (-1.)
    }
}
#[derive(Clone, Debug)]
pub(super) struct S {
    pub line: bool,
    pub quad: bool,
    pub points: [P; 3],
}
#[derive(Clone)]
struct F {
    segment: S,
    first: usize,
    last: usize,
    t1: P,
    t2: P,
}
fn distance_to_line(p: P, a: P, b: P) -> f64 {
    let v = b - a;
    let l = v.len();
    if l == 0. {
        (p - a).len()
    } else {
        (p - a).cross(v).abs() / l
    }
}
fn distance_to_segment(p: P, a: P, b: P) -> f64 {
    let v = b - a;
    let l = v.dot(v);
    let t = if l == 0. { 0. } else { (p - a).dot(v) / l }.clamp(0., 1.);
    (p - (a + v * t)).len()
}
fn dp(points: &[P], from: usize, to: usize, error: f64) -> Vec<usize> {
    let n = points.len();
    let mut keep = Vec::new();
    let mut stack = vec![(from, to)];
    while let Some((a, b)) = stack.pop() {
        let mut worst = None;
        let mut distance = error;
        for i in a + 1..b {
            let d = distance_to_line(points[i % n], points[a % n], points[b % n]);
            if d > distance {
                distance = d;
                worst = Some(i);
            }
        }
        if let Some(i) = worst {
            keep.push(i % n);
            stack.push((a, i));
            stack.push((i, b));
        }
    }
    keep.sort_unstable();
    keep
}
fn simplify(points: &[P], error: f64, lattice: bool, band: f64) -> Vec<usize> {
    let n = points.len();
    if n <= 3 || !lattice && error <= 0. {
        return (0..n).collect();
    }
    if lattice {
        let straight = |a: usize, b: usize| {
            if (points[b] - points[a]).len() == 0. {
                return false;
            }
            (a + 1..b).all(|k| distance_to_line(points[k], points[a], points[b]) <= band)
        };
        let mut anchors = vec![0];
        let mut i = 0;
        while i < n - 1 {
            let mut step = 1;
            while i + step * 2 < n && straight(i, i + step * 2) {
                step *= 2;
            }
            let (mut lo, mut hi) = (i + step, (n - 1).min(i + step * 2));
            while lo < hi {
                let mid = (lo + hi + 1) / 2;
                if straight(i, mid) {
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            i = lo.max(i + 1);
            anchors.push(i);
        }
        if anchors.last() == Some(&(n - 1)) {
            anchors.pop();
        }
        while anchors.len() < 3 && anchors.len() < n {
            let (at, gap) = anchors
                .iter()
                .enumerate()
                .map(|(i, &a)| (i, anchors.get(i + 1).copied().unwrap_or(n) - a))
                .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
                .unwrap();
            if gap < 2 {
                break;
            }
            anchors.insert(at + 1, anchors[at] + gap / 2);
        }
        return anchors;
    }
    let far = (1..n)
        .max_by(|&a, &b| {
            (points[a] - points[0])
                .dot(points[a] - points[0])
                .total_cmp(&(points[b] - points[0]).dot(points[b] - points[0]))
                .then_with(|| b.cmp(&a))
        })
        .unwrap_or(0);
    let mut out = vec![0];
    out.extend(dp(points, 0, far, error));
    out.push(far);
    // Wrapped DP order must follow the second chain, not numerical index order.
    let mut second = dp(points, far, n, error);
    second.sort_by_key(|&i| if i < far { i + n } else { i });
    out.extend(second);
    out.dedup();
    out
}
fn regularise(points: &mut [P], passes: u8, band: f64) {
    if points.len() < 3 || passes == 0 || band <= 0. {
        return;
    }
    let original = points.to_vec();
    let n = points.len();
    for _ in 0..passes {
        let next: Vec<_> = (0..n)
            .map(|i| {
                let p = (points[i] + (points[(i + n - 1) % n] + points[(i + 1) % n]) * 0.5) * 0.5;
                let v = p - original[i];
                if v.len() > band {
                    original[i] + v * (band / v.len())
                } else {
                    p
                }
            })
            .collect();
        points.copy_from_slice(&next);
    }
}
fn tangent(points: &[P], i: usize, left: bool) -> P {
    let k = 4.min(if left { points.len() - 1 - i } else { i });
    let mut v = P::new(0., 0.);
    for j in 1..=k {
        v = v + (points[if left { i + j } else { i - j }] - points[i]) * (1. / j as f64);
    }
    v.norm()
}
fn center(points: &[P], i: usize, closed: bool) -> P {
    let n = points.len();
    let k = if closed {
        4.min(n / 2 - 1).max(1)
    } else {
        4.min(i).min(n - 1 - i)
    };
    if closed {
        (points[(i + k) % n] - points[(i + n - k) % n]).norm()
    } else {
        (points[i + k.max(1)] - points[i - k.max(1)]).norm()
    }
}
fn parameters(points: &[P], a: usize, b: usize) -> Vec<f64> {
    let mut u = vec![0.; b - a + 1];
    for i in a + 1..=b {
        u[i - a] = u[i - a - 1] + (points[i] - points[i - 1]).len();
    }
    let total = u[b - a];
    if total > 0. {
        for v in &mut u {
            *v /= total;
        }
    }
    u
}
fn basis(t: f64) -> [f64; 4] {
    let m = 1. - t;
    [m * m * m, 3. * t * m * m, 3. * t * t * m, t * t * t]
}
fn bezier(p: [P; 4], t: f64) -> P {
    let b = basis(t);
    p[0] * b[0] + p[1] * b[1] + p[2] * b[2] + p[3] * b[3]
}
fn generate(points: &[P], a: usize, b: usize, u: &[f64], t1: P, t2: P) -> [P; 4] {
    let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0., 0., 0., 0., 0.);
    for i in 0..u.len() {
        let v = basis(u[i]);
        let a0 = t1 * v[1];
        let a1 = t2 * v[2];
        c00 += a0.dot(a0);
        c01 += a0.dot(a1);
        c11 += a1.dot(a1);
        let residual = points[a + i] - (points[a] * (v[0] + v[1]) + points[b] * (v[2] + v[3]));
        x0 += a0.dot(residual);
        x1 += a1.dot(residual);
    }
    let det = c00 * c11 - c01 * c01;
    let (mut left, mut right) = if det == 0. {
        (0., 0.)
    } else {
        ((x0 * c11 - c01 * x1) / det, (c00 * x1 - c01 * x0) / det)
    };
    let length = (points[b] - points[a]).len();
    if left < 1e-6 * length || right < 1e-6 * length {
        left = length / 3.;
        right = left;
    }
    left = left.min(length);
    right = right.min(length);
    [
        points[a],
        points[a] + t1 * left,
        points[b] + t2 * right,
        points[b],
    ]
}
fn sampled(points: &[P], a: usize, b: usize, bez: [P; 4], u: &[f64]) -> (f64, usize) {
    let mut max = 0.;
    let mut split = a + (b - a + 1) / 2;
    for i in a + 1..b {
        let v = bezier(bez, u[i - a]) - points[i];
        let d = v.dot(v);
        if d >= max {
            max = d;
            split = i;
        }
    }
    (max.sqrt(), split)
}
fn continuous(points: &[P], a: usize, b: usize, bez: [P; 4]) -> f64 {
    let n = ((b - a) * 4).clamp(20, 800);
    let mut worst = 0f64;
    for k in 0..=n {
        let p = bezier(bez, k as f64 / n as f64);
        let best = (a..b)
            .map(|i| distance_to_segment(p, points[i], points[i + 1]))
            .fold(f64::INFINITY, f64::min);
        worst = worst.max(best);
    }
    worst
}
fn newton(bez: [P; 4], p: P, u: f64) -> f64 {
    let q = bezier(bez, u);
    let h = [
        (bez[1] - bez[0]) * 3.,
        (bez[2] - bez[1]) * 3.,
        (bez[3] - bez[2]) * 3.,
    ];
    let m = 1. - u;
    let d1 = h[0] * (m * m) + h[1] * (2. * m * u) + h[2] * (u * u);
    let d2 = (h[1] - h[0]) * (2. * m) + (h[2] - h[1]) * (2. * u);
    let denom = d1.dot(d1) + (q - p).dot(d2);
    if denom == 0. {
        u
    } else {
        u - (q - p).dot(d1) / denom
    }
}
fn emit(bez: [P; 4], a: usize, b: usize, t1: P, t2: P) -> F {
    let line = (bez[3] - bez[0]).len() > 0.
        && distance_to_line(bez[1], bez[0], bez[3]) < 0.05
        && distance_to_line(bez[2], bez[0], bez[3]) < 0.05;
    F {
        segment: S {
            quad: false,
            line,
            points: if line {
                [bez[0], bez[3], bez[3]]
            } else {
                [bez[1], bez[2], bez[3]]
            },
        },
        first: a,
        last: b,
        t1,
        t2,
    }
}
fn cubic(
    points: &[P],
    a: usize,
    b: usize,
    t1: P,
    t2: P,
    error: f64,
    out: &mut Vec<F>,
    depth: usize,
    c: &WorkControl,
) -> Result<(), String> {
    c.check()?;
    if b == a + 1 {
        let length = (points[b] - points[a]).len() / 3.;
        out.push(emit(
            [
                points[a],
                points[a] + t1 * length,
                points[b] + t2 * length,
                points[b],
            ],
            a,
            b,
            t1,
            t2,
        ));
        return Ok(());
    }
    let mut u = parameters(points, a, b);
    let mut bez = generate(points, a, b, &u, t1, t2);
    let (mut max, mut split) = sampled(points, a, b, bez, &u);
    if max < error && continuous(points, a, b, bez) < error {
        out.push(emit(bez, a, b, t1, t2));
        return Ok(());
    }
    if max < error * 4. {
        for _ in 0..6 {
            let next: Vec<_> = u
                .iter()
                .enumerate()
                .map(|(i, &v)| newton(bez, points[a + i], v))
                .collect();
            bez = generate(points, a, b, &next, t1, t2);
            (max, split) = sampled(points, a, b, bez, &next);
            if max < error && continuous(points, a, b, bez) < error {
                out.push(emit(bez, a, b, t1, t2));
                return Ok(());
            }
            u = next;
        }
    }
    if depth > 24 {
        // Preserve the error contract rather than emit an unverified curve.
        for i in a + 1..=b {
            out.push(F {
                segment: S {
                    quad: false,
                    line: true,
                    points: [points[i - 1], points[i], points[i]],
                },
                first: i - 1,
                last: i,
                t1,
                t2,
            });
        }
        return Ok(());
    }
    if split <= a || split >= b {
        split = a + ((b - a) / 2).max(1);
    }
    let t = center(points, split, false);
    cubic(points, a, split, t1, -t, error, out, depth + 1, c)?;
    cubic(points, split, b, t, t2, error, out, depth + 1, c)
}

fn merge(points: &[P], mut curves: Vec<F>, error: f64, c: &WorkControl) -> Result<Vec<F>, String> {
    for _ in 0..8 {
        c.check()?;
        let mut next = Vec::new();
        let mut changed = false;
        let mut i = 0;
        while i < curves.len() {
            let a = &curves[i];
            if let Some(b) = curves.get(i + 1) {
                if a.last == b.first && b.last - a.first >= 2 {
                    let u = parameters(points, a.first, b.last);
                    let bez = generate(points, a.first, b.last, &u, a.t1, b.t2);
                    if sampled(points, a.first, b.last, bez, &u).0 <= error
                        && continuous(points, a.first, b.last, bez) <= error
                    {
                        next.push(emit(bez, a.first, b.last, a.t1, b.t2));
                        changed = true;
                        i += 2;
                        continue;
                    }
                }
            }
            next.push(a.clone());
            i += 1;
        }
        curves = next;
        if !changed {
            break;
        }
    }
    Ok(curves)
}
pub(super) fn closed(input: &[P], o: &Options, c: &WorkControl) -> Result<Vec<S>, String> {
    if input.len() < 3 {
        return Ok(Vec::new());
    }
    let _scratch = c.reserve(input.len() * 160)?;
    let mut points = input.to_vec();
    regularise(&mut points, o.vc_regularise, o.vc_regularise_band);
    let mut tolerance = o.vc_tolerance;
    let mut anchors = simplify(&points, tolerance, o.vc_lattice_simplify, o.vc_lattice_band);
    while !o.vc_lattice_simplify && anchors.len() < 3 && tolerance > 0. {
        tolerance = if tolerance > 0.05 { tolerance / 2. } else { 0. };
        anchors = simplify(&points, tolerance, false, o.vc_lattice_band);
    }
    if anchors.len() < 3 {
        return Ok(Vec::new());
    }
    let mut forced = vec![false; points.len()];
    let m = anchors.len();
    if o.vc_right_angle {
        let slack = o.vc_right_angle_threshold.to_radians().sin();
        for i in 0..m {
            let (a, b, d) = (anchors[(i + m - 1) % m], anchors[i], anchors[(i + 1) % m]);
            let vin = points[b] - points[a];
            let vout = points[d] - points[b];
            let (la, lb) = (vin.len(), vout.len());
            if la == 0. || lb == 0. || vin.dot(vout).abs() / (la * lb) > slack {
                continue;
            }
            if vin.y.abs() / la <= slack && vout.x.abs() / lb <= slack {
                points[b] = P::new(points[d].x, points[a].y);
                forced[b] = true;
            } else if vin.x.abs() / la <= slack && vout.y.abs() / lb <= slack {
                points[b] = P::new(points[a].x, points[d].y);
                forced[b] = true;
            }
        }
    }
    let mut output = Vec::new();
    let n = points.len();
    if o.vc_polygon_only || anchors.len() == n && !o.vc_right_angle {
        for i in 1..=m {
            let p = points[anchors[i % m]];
            output.push(S {
                quad: false,
                line: true,
                points: [p, p, p],
            });
        }
        return Ok(output);
    }
    let threshold = o.vc_corner_angle.to_radians().cos();
    let mut breaks = Vec::new();
    for i in 0..m {
        let a = points[anchors[i]] - points[anchors[(i + m - 1) % m]];
        let b = points[anchors[(i + 1) % m]] - points[anchors[i]];
        if a.len() > 0.
            && b.len() > 0.
            && (a.dot(b) / (a.len() * b.len()) < threshold || forced[anchors[i]])
        {
            breaks.push((anchors[i], true));
        }
    }
    if breaks.is_empty() {
        breaks.push((anchors[0], false));
        breaks.push((anchors[m / 2], false));
    } else if breaks.len() == 1 {
        let i = anchors.iter().position(|&a| a == breaks[0].0).unwrap();
        breaks.push((anchors[(i + m / 2) % m], false));
    }
    for i in 0..breaks.len() {
        let (a, corner_a) = breaks[i];
        let (b, corner_b) = breaks[(i + 1) % breaks.len()];
        let count = if a <= b { b - a + 1 } else { n - a + b + 1 };
        let chain: Vec<_> = (0..count).map(|k| points[(a + k) % n]).collect();
        let t1 = if corner_a {
            tangent(&chain, 0, true)
        } else {
            center(&points, a, true)
        };
        let t2 = if corner_b {
            tangent(&chain, chain.len() - 1, false)
        } else {
            -center(&points, b, true)
        };
        let mut fitted = Vec::new();
        cubic(
            &chain,
            0,
            chain.len() - 1,
            t1,
            t2,
            o.vc_fit_error,
            &mut fitted,
            0,
            c,
        )?;
        if o.vc_optimize {
            fitted = merge(&chain, fitted, o.vc_optimize_error, c)?;
        }
        if o.vc_quadratics {
            quadratics(
                &chain,
                &mut fitted,
                if o.vc_optimize {
                    o.vc_fit_error.max(o.vc_optimize_error)
                } else {
                    o.vc_fit_error
                },
            );
        }
        output.extend(fitted.into_iter().map(|f| f.segment));
    }
    Ok(output)
}

fn quadratics(chain: &[P], fitted: &mut [F], error: f64) {
    for f in fitted {
        if f.segment.line {
            continue;
        }
        let [c1, c2, p3] = f.segment.points;
        let p0 = chain[f.first];
        let q = ((c1 + c2) * 3. - p0 - p3) * 0.25;
        let reduced = [p0, p0 + (q - p0) * (2. / 3.), p3 + (q - p3) * (2. / 3.), p3];
        let u = parameters(chain, f.first, f.last);
        let mut worst = 0f64;
        for i in f.first + 1..f.last {
            let p = chain[i];
            let mut t = u[i - f.first];
            let mut dist = (bezier(reduced, t) - p).len();
            for _ in 0..4 {
                let next = newton(reduced, p, t).clamp(0., 1.);
                let d = (bezier(reduced, next) - p).len();
                if d > dist {
                    break;
                }
                t = next;
                dist = d;
            }
            worst = worst.max(dist);
        }
        let ok = if f.last - f.first >= 3 {
            worst <= error && continuous(chain, f.first, f.last, reduced) <= error
        } else {
            let deviation = (-p0 + c1 * 3. - c2 * 3. + p3).len() * 3f64.sqrt() / 36.;
            sampled(chain, f.first, f.last, [p0, c1, c2, p3], &u)
                .0
                .max(continuous(chain, f.first, f.last, [p0, c1, c2, p3]))
                + deviation
                <= error
        };
        if ok {
            f.segment.quad = true;
            f.segment.points = [reduced[1], reduced[2], p3];
        }
    }
}

pub(super) fn open(input: &[P], o: &Options, c: &WorkControl) -> Result<Vec<S>, String> {
    if input.len() < 2 {
        return Ok(Vec::new());
    }
    let _scratch = c.reserve(input.len() * 160)?;
    let mut p = input.to_vec();
    let original = p.clone();
    let n = p.len();
    for _ in 0..o.vc_regularise {
        c.check()?;
        let mut next = p.clone();
        for i in 1..n - 1 {
            let v = (p[i] + (p[i - 1] + p[i + 1]) * 0.5) * 0.5;
            let d = v - original[i];
            next[i] = if d.len() > o.vc_regularise_band {
                original[i] + d * (o.vc_regularise_band / d.len())
            } else {
                v
            };
        }
        p = next;
    }
    let mut anchors = vec![0];
    anchors.extend(dp(&p, 0, n - 1, o.vc_tolerance));
    anchors.push(n - 1);
    if anchors.len() == 2 || o.vc_polygon_only {
        return Ok(anchors[1..]
            .iter()
            .map(|&i| S {
                line: true,
                quad: false,
                points: [p[i]; 3],
            })
            .collect());
    }
    let mut breaks = vec![0];
    let threshold = o.vc_corner_angle.to_radians().cos();
    for i in 1..anchors.len() - 1 {
        let a = p[anchors[i]] - p[anchors[i - 1]];
        let b = p[anchors[i + 1]] - p[anchors[i]];
        if a.len() > 0. && b.len() > 0. && a.dot(b) / (a.len() * b.len()) < threshold {
            breaks.push(anchors[i]);
        }
    }
    breaks.push(n - 1);
    let mut output = Vec::new();
    for pair in breaks.windows(2) {
        let chain = &p[pair[0]..=pair[1]];
        let mut fitted = Vec::new();
        cubic(
            chain,
            0,
            chain.len() - 1,
            tangent(chain, 0, true),
            tangent(chain, chain.len() - 1, false),
            o.vc_fit_error,
            &mut fitted,
            0,
            c,
        )?;
        if o.vc_optimize {
            fitted = merge(chain, fitted, o.vc_optimize_error, c)?;
        }
        if o.vc_quadratics {
            quadratics(chain, &mut fitted, o.vc_fit_error.max(o.vc_optimize_error));
        }
        output.extend(fitted.into_iter().map(|f| f.segment));
    }
    Ok(output)
}
pub(super) fn reverse(start: P, segments: &[S]) -> Vec<S> {
    let mut positions = vec![start];
    positions.extend(segments.iter().map(|s| s.points[2]));
    segments
        .iter()
        .enumerate()
        .rev()
        .map(|(i, s)| S {
            line: s.line,
            quad: s.quad,
            points: if s.line {
                [positions[i]; 3]
            } else {
                [s.points[1], s.points[0], positions[i]]
            },
        })
        .collect()
}
