//! 根据 Peter Selinger 的 Potrace 1.16 算法重写的安全 Rust 实现。
//! 原作者 Copyright (C) 2001–2019 Peter Selinger，GPL-2.0-or-later。
//! 使用拥有所有权的数组表示轮廓、累积矩和曲线，没有 C ABI 或外部进程。
use crate::control::WorkControl;
use std::fmt::Write;

#[derive(Clone, Copy, Default, Debug)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
impl Point {
    fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    fn sub(self, b: Self) -> Self {
        Self::new(self.x - b.x, self.y - b.y)
    }
    fn cross(self, b: Self) -> f64 {
        self.x * b.y - self.y * b.x
    }
    fn dot(self, b: Self) -> f64 {
        self.x * b.x + self.y * b.y
    }
    fn distance(self, b: Self) -> f64 {
        ((self.x - b.x) * (self.x - b.x) + (self.y - b.y) * (self.y - b.y)).sqrt()
    }
    fn lerp(self, b: Self, t: f64) -> Self {
        Self::new(self.x + t * (b.x - self.x), self.y + t * (b.y - self.y))
    }
}
fn para(a: Point, b: Point, c: Point) -> f64 {
    b.sub(a).cross(c.sub(a))
}
fn sign(x: f64) -> f64 {
    if x > 0. {
        1.
    } else if x < 0. {
        -1.
    } else {
        0.
    }
}
fn cyclic(a: usize, b: usize, c: usize) -> bool {
    if a <= c {
        a <= b && b < c
    } else {
        a <= b || b < c
    }
}

// 原始分解算法采用从下往上的笛卡尔坐标；SVG 输出时再翻转 y。
#[derive(Clone)]
struct Bitmap {
    width: usize,
    height: usize,
    words: Vec<u64>,
    stride: usize,
}
impl Bitmap {
    fn new(mask: &[bool], width: usize, height: usize) -> Self {
        let stride = width.div_ceil(64);
        let mut bitmap = Self {
            width,
            height,
            stride,
            words: vec![0; stride * height],
        };
        for y in 0..height {
            for x in 0..width {
                if mask[(height - 1 - y) * width + x] {
                    bitmap.words[y * stride + x / 64] |= 1 << (63 - x % 64);
                }
            }
        }
        bitmap
    }
    fn get(&self, x: i64, y: i64) -> bool {
        x >= 0
            && y >= 0
            && (x as usize) < self.width
            && (y as usize) < self.height
            && self.words[y as usize * self.stride + x as usize / 64]
                & (1 << (63 - x as usize % 64))
                != 0
    }
    fn majority(&self, x: i64, y: i64) -> bool {
        for radius in 2..5 {
            let mut count = 0;
            for a in -radius + 1..=radius - 1 {
                for (px, py) in [
                    (x + a, y + radius - 1),
                    (x + radius - 1, y + a - 1),
                    (x + a - 1, y - radius),
                    (x - radius, y + a),
                ] {
                    count += if self.get(px, py) { 1 } else { -1 };
                }
            }
            if count != 0 {
                return count > 0;
            }
        }
        false
    }
    fn xor_prefix(&mut self, x: usize, y: usize) {
        let row = y * self.stride;
        for word in 0..x / 64 {
            self.words[row + word] ^= u64::MAX;
        }
        if x % 64 != 0 {
            self.words[row + x / 64] ^= u64::MAX << (64 - x % 64);
        }
    }
}

#[derive(Clone, Default, Debug)]
pub struct Segment {
    pub corner: bool,
    pub quadratic: bool,
    pub controls: [Point; 3],
    vertex: Point,
    alpha: f64,
}
impl Segment {
    pub(crate) fn from_controls(corner: bool, controls: [Point; 3]) -> Self {
        Self {
            corner,
            quadratic: false,
            controls,
            vertex: controls[2],
            alpha: 0.,
        }
    }
}
pub type Curve = Vec<Segment>;

pub fn trace(
    mask: &[bool],
    width: usize,
    height: usize,
    despeckle: usize,
    smooth: f64,
    tolerance: f64,
) -> Vec<Curve> {
    trace_with_parallelism(
        mask,
        width,
        height,
        despeckle,
        smooth,
        tolerance,
        &crate::parallel::Parallelism::new(1),
    )
}
pub fn trace_with_parallelism(
    mask: &[bool],
    width: usize,
    height: usize,
    despeckle: usize,
    smooth: f64,
    tolerance: f64,
    workers: &crate::parallel::Parallelism,
) -> Vec<Curve> {
    trace_controlled(
        mask,
        width,
        height,
        despeckle,
        smooth,
        tolerance,
        workers,
        &WorkControl::unlimited(),
    )
    .unwrap_or_default()
}
pub fn trace_controlled(
    mask: &[bool],
    width: usize,
    height: usize,
    despeckle: usize,
    smooth: f64,
    tolerance: f64,
    workers: &crate::parallel::Parallelism,
    control: &WorkControl,
) -> Result<Vec<Curve>, String> {
    control.check()?;
    if width == 0 || height == 0 || mask.len() != width * height {
        return Ok(Vec::new());
    }
    let original = Bitmap::new(mask, width, height);
    let mut bitmap = original.clone();
    let mut result = Vec::new();
    // 搜索顺序为自上而下，每行从左至右，保持原始算法的起点选择。
    let mut row = height as i64 - 1;
    while row >= 0 {
        control.check()?;
        let mut found = None;
        while row >= 0 && found.is_none() {
            if row & 255 == 0 {
                control.check()?;
            }
            for word in 0..bitmap.stride {
                let bits = bitmap.words[row as usize * bitmap.stride + word];
                if bits != 0 {
                    found = Some(word * 64 + bits.leading_zeros() as usize);
                    break;
                }
            }
            if found.is_none() {
                row -= 1;
            }
        }
        let Some(x0) = found else {
            break;
        };
        let y0 = row + 1;
        let negative = !original.get(x0 as i64, row);
        let (mut x, mut y, mut dx, mut dy) = (x0 as i64, y0, 0i64, -1i64);
        let mut points = Vec::new();
        let mut area = 0i64;
        loop {
            if points.len() & 4095 == 0 {
                control.check()?;
            }
            if points.len() == points.capacity() {
                let grow = points.capacity().max(8);
                // Charge conservatively for points, fitting scratch, retained curves and preview paths.
                control.claim(grow * 384)?;
                points
                    .try_reserve_exact(grow)
                    .map_err(|_| "轮廓内存分配失败")?;
            }
            points.push(Point::new(x as f64, y as f64));
            x += dx;
            y += dy;
            area += x * dy;
            if x == x0 as i64 && y == y0 {
                break;
            }
            let left = bitmap.get(x + (dx + dy - 1) / 2, y + (dy - dx - 1) / 2);
            let right = bitmap.get(x + (dx - dy - 1) / 2, y + (dy + dx - 1) / 2);
            if left && !right {
                if !bitmap.majority(x, y) {
                    (dx, dy) = (dy, -dx);
                } else {
                    (dx, dy) = (-dy, dx);
                }
            } else if left {
                (dx, dy) = (dy, -dx);
            } else if !right {
                (dx, dy) = (-dy, dx);
            }
        }
        let mut previous_y = points.last().unwrap().y as usize;
        for (index, point) in points.iter().enumerate() {
            if index & 8191 == 0 && control.cancelled() {
                break;
            }
            let current_y = point.y as usize;
            if current_y != previous_y {
                bitmap.xor_prefix(point.x as usize, current_y.min(previous_y));
                previous_y = current_y;
            }
        }
        if area <= despeckle as i64 {
            continue;
        }
        result.push((points, negative));
    }
    let curves = workers.map(&result, |(points, negative)| -> Result<Curve, String> {
        control.check()?;
        let path = Path::new(points, control);
        control.check()?;
        let polygon = path.polygon();
        control.check()?;
        let mut vertices = path.adjust(&polygon);
        control.check()?;
        if *negative {
            vertices.reverse();
        }
        let mut curve = smooth_curve(&vertices, smooth);
        if tolerance > 0. {
            if smooth <= 0. || curve.iter().all(|segment| segment.corner) {
                // No Bezier curves can merge. Preserve the old closed-contour
                // starting point without allocating the optimizer's scratch arrays.
                if curve.len() >= 2 {
                    curve.rotate_left(1);
                }
                control.check()?;
                return Ok(curve);
            }
            let curve = optimize(&curve, tolerance, control);
            control.check()?;
            Ok(curve)
        } else {
            Ok(curve)
        }
    });
    curves.into_iter().collect()
}

struct Path<'a> {
    points: &'a [Point],
    control: &'a WorkControl,
    sums: Vec<[f64; 5]>,
}
impl<'a> Path<'a> {
    fn new(points: &'a [Point], control: &'a WorkControl) -> Self {
        let origin = points[0];
        let mut sums = vec![[0.; 5]];
        for (index, point) in points.iter().enumerate() {
            if index & 8191 == 0 && control.cancelled() {
                break;
            }
            let p = point.sub(origin);
            let values = [p.x, p.y, p.x * p.x, p.x * p.y, p.y * p.y];
            let previous = sums.last().unwrap();
            sums.push(std::array::from_fn(|k| previous[k] + values[k]));
        }
        Self {
            points,
            sums,
            control,
        }
    }
    fn moments(&self, i: usize, j: usize) -> ([f64; 5], f64) {
        let n = self.points.len();
        let r = j / n - i / n;
        let (i, j) = (i % n, j % n);
        (
            std::array::from_fn(|k| {
                self.sums[j + 1][k] - self.sums[i][k] + r as f64 * self.sums[n][k]
            }),
            (j + 1 + r * n - i) as f64,
        )
    }
    fn longest(&self) -> Vec<usize> {
        let p = &self.points;
        let n = p.len();
        let mut next = vec![0; n];
        let mut k = 0;
        for i in (0..n).rev() {
            if i & 255 == 0 && self.control.cancelled() {
                return Vec::new();
            }
            if p[i].x != p[k].x && p[i].y != p[k].y {
                k = i + 1;
            }
            next[i] = k;
        }
        let mut pivot = vec![0; n];
        for i in (0..n).rev() {
            if i & 255 == 0 && self.control.cancelled() {
                return Vec::new();
            }
            let mut directions = [0; 4];
            let delta = p[(i + 1) % n].sub(p[i]);
            directions[((3. + 3. * delta.x + delta.y) / 2.) as usize] += 1;
            let mut constraints = [Point::default(); 2];
            let mut k = next[i];
            let mut k1 = i;
            let mut all_directions = false;
            loop {
                let delta = p[k].sub(p[k1]);
                directions[((3. + 3. * sign(delta.x) + sign(delta.y)) / 2.) as usize] += 1;
                if directions.iter().all(|&v| v != 0) {
                    pivot[i] = k1;
                    all_directions = true;
                    break;
                }
                let cur = p[k].sub(p[i]);
                if constraints[0].cross(cur) < 0. || constraints[1].cross(cur) > 0. {
                    break;
                }
                if cur.x.abs() > 1. || cur.y.abs() > 1. {
                    let off = Point::new(
                        cur.x
                            + if cur.y >= 0. && (cur.y > 0. || cur.x < 0.) {
                                1.
                            } else {
                                -1.
                            },
                        cur.y
                            + if cur.x <= 0. && (cur.x < 0. || cur.y < 0.) {
                                1.
                            } else {
                                -1.
                            },
                    );
                    if constraints[0].cross(off) >= 0. {
                        constraints[0] = off;
                    }
                    let off = Point::new(
                        cur.x
                            + if cur.y <= 0. && (cur.y < 0. || cur.x < 0.) {
                                1.
                            } else {
                                -1.
                            },
                        cur.y
                            + if cur.x >= 0. && (cur.x > 0. || cur.y < 0.) {
                                1.
                            } else {
                                -1.
                            },
                    );
                    if constraints[1].cross(off) <= 0. {
                        constraints[1] = off;
                    }
                }
                k1 = k;
                k = next[k1];
                if !cyclic(k, i, k1) {
                    break;
                }
            }
            if !all_directions {
                let delta = p[k].sub(p[k1]);
                let dk = Point::new(sign(delta.x), sign(delta.y));
                let cur = p[k1].sub(p[i]);
                let (a, b, c, d) = (
                    constraints[0].cross(cur),
                    constraints[0].cross(dk),
                    constraints[1].cross(cur),
                    constraints[1].cross(dk),
                );
                let mut j = 10_000_000i64;
                if b < 0. {
                    j = (a / -b).floor() as i64;
                }
                if d > 0. {
                    j = j.min((-c / d).floor() as i64);
                }
                pivot[i] = (k1 as i64 + j).rem_euclid(n as i64) as usize;
            }
        }
        let mut lon = vec![0; n];
        let mut j = pivot[n - 1];
        lon[n - 1] = j;
        for i in (0..n - 1).rev() {
            if cyclic(i + 1, pivot[i], j) {
                j = pivot[i];
            }
            lon[i] = j;
        }
        for i in (0..n).rev() {
            if i & 255 == 0 && self.control.cancelled() {
                return Vec::new();
            }
            if !cyclic((i + 1) % n, j, lon[i]) {
                break;
            }
            lon[i] = j;
        }
        lon
    }
    fn penalty(&self, i: usize, j: usize) -> f64 {
        let ([x, y, x2, xy, y2], k) = self.moments(i, j);
        let p = self.points[i];
        let q = self.points[j % self.points.len()];
        let mid = p.lerp(q, 0.5).sub(self.points[0]);
        let ex = -(q.y - p.y);
        let ey = q.x - p.x;
        let a = (x2 - 2. * x * mid.x) / k + mid.x * mid.x;
        let b = (xy - x * mid.y - y * mid.x) / k + mid.x * mid.y;
        let c = (y2 - 2. * y * mid.y) / k + mid.y * mid.y;
        (ex * ex * a + 2. * ex * ey * b + ey * ey * c)
            .max(0.)
            .sqrt()
    }
    fn polygon(&self) -> Vec<usize> {
        let n = self.points.len();
        let lon = self.longest();
        if self.control.cancelled() {
            return Vec::new();
        }
        let mut clip0 = vec![0; n];
        let mut clip1 = vec![0; n + 1];
        for i in 0..n {
            let mut c = (lon[(i + n - 1) % n] + n - 1) % n;
            if c == i {
                c = (i + 1) % n;
            }
            clip0[i] = if c < i { n } else { c };
        }
        let mut j = 1;
        for (i, &c) in clip0.iter().enumerate() {
            while j <= c {
                clip1[j] = i;
                j += 1;
            }
        }
        let mut forward = vec![0];
        let mut i = 0;
        while i < n {
            i = clip0[i];
            forward.push(i);
        }
        let m = forward.len() - 1;
        let mut backward = vec![0; m + 1];
        let mut i = n;
        for j in (1..=m).rev() {
            if self.control.cancelled() {
                return Vec::new();
            }
            backward[j] = i;
            i = clip1[i];
        }
        let mut penalty = vec![0.; n + 1];
        let mut previous = vec![0; n + 1];
        for j in 1..=m {
            for i in backward[j]..=forward[j] {
                let mut best = f64::INFINITY;
                for k in (clip1[i]..=forward[j - 1]).rev() {
                    if k & 511 == 0 && self.control.cancelled() {
                        return Vec::new();
                    }
                    let value = self.penalty(k, i) + penalty[k];
                    if value < best {
                        best = value;
                        previous[i] = k;
                    }
                }
                penalty[i] = best;
            }
        }
        let mut polygon = vec![0; m];
        let mut i = n;
        for j in (0..m).rev() {
            i = previous[i];
            polygon[j] = i;
        }
        polygon
    }
    fn slope(&self, i: usize, j: usize) -> (Point, Point) {
        let ([x, y, x2, xy, y2], k) = self.moments(i, j);
        let center = Point::new(x / k, y / k);
        let mut a = (x2 - x * x / k) / k;
        let b = (xy - x * y / k) / k;
        let mut c = (y2 - y * y / k) / k;
        let lambda = (a + c + ((a - c) * (a - c) + 4. * b * b).sqrt()) / 2.;
        a -= lambda;
        c -= lambda;
        let direction = if a.abs() >= c.abs() {
            Point::new(-b, a)
        } else {
            Point::new(-c, b)
        };
        let length = (direction.x * direction.x + direction.y * direction.y).sqrt();
        (
            center,
            if length != 0. {
                Point::new(direction.x / length, direction.y / length)
            } else {
                Point::default()
            },
        )
    }
    fn adjust(&self, polygon: &[usize]) -> Vec<Point> {
        let m = polygon.len();
        let n = self.points.len();
        let mut forms = vec![[[0.; 3]; 3]; m];
        for i in 0..m {
            let start = polygon[i];
            let end = start + (polygon[(i + 1) % m] + n - start) % n;
            let (center, direction) = self.slope(start, end);
            let d = direction.dot(direction);
            if d != 0. {
                let v = [
                    direction.y,
                    -direction.x,
                    direction.x * center.y - direction.y * center.x,
                ];
                for l in 0..3 {
                    for k in 0..3 {
                        forms[i][l][k] = v[l] * v[k] / d;
                    }
                }
            }
        }
        let origin = self.points[0];
        let mut vertices = Vec::with_capacity(m);
        for i in 0..m {
            let s = self.points[polygon[i]].sub(origin);
            let mut q = [[0.; 3]; 3];
            for l in 0..3 {
                for k in 0..3 {
                    q[l][k] = forms[(i + m - 1) % m][l][k] + forms[i][l][k];
                }
            }
            let mut w;
            loop {
                let det = q[0][0] * q[1][1] - q[0][1] * q[1][0];
                if det != 0. {
                    w = Point::new(
                        (-q[0][2] * q[1][1] + q[1][2] * q[0][1]) / det,
                        (q[0][2] * q[1][0] - q[1][2] * q[0][0]) / det,
                    );
                    break;
                }
                let (vx, vy) = if q[0][0] > q[1][1] {
                    (-q[0][1], q[0][0])
                } else if q[1][1] != 0. {
                    (-q[1][1], q[1][0])
                } else {
                    (1., 0.)
                };
                let v = [vx, vy, -vy * s.y - vx * s.x];
                let d = vx * vx + vy * vy;
                for l in 0..3 {
                    for k in 0..3 {
                        q[l][k] += v[l] * v[k] / d;
                    }
                }
            }
            if (w.x - s.x).abs() <= 0.5 && (w.y - s.y).abs() <= 0.5 {
                vertices.push(Point::new(w.x + origin.x, w.y + origin.y));
                continue;
            }
            let mut best = s;
            let mut minimum = quadratic(&q, s);
            let mut candidates = Vec::new();
            for z in [-0.5, 0.5] {
                if q[0][0] != 0. {
                    let y = s.y + z;
                    let x = -(q[0][1] * y + q[0][2]) / q[0][0];
                    if (x - s.x).abs() <= 0.5 {
                        candidates.push(Point::new(x, y));
                    }
                }
                if q[1][1] != 0. {
                    let x = s.x + z;
                    let y = -(q[1][0] * x + q[1][2]) / q[1][1];
                    if (y - s.y).abs() <= 0.5 {
                        candidates.push(Point::new(x, y));
                    }
                }
            }
            for dx in [-0.5, 0.5] {
                for dy in [-0.5, 0.5] {
                    candidates.push(Point::new(s.x + dx, s.y + dy));
                }
            }
            for candidate in candidates {
                let value = quadratic(&q, candidate);
                if value < minimum {
                    minimum = value;
                    best = candidate;
                }
            }
            w = best;
            vertices.push(Point::new(w.x + origin.x, w.y + origin.y));
        }
        vertices
    }
}
fn quadratic(q: &[[f64; 3]; 3], p: Point) -> f64 {
    let v = [p.x, p.y, 1.];
    let mut value = 0.;
    for l in 0..3 {
        for k in 0..3 {
            value += v[l] * q[l][k] * v[k];
        }
    }
    value
}
fn smooth_curve(vertices: &[Point], maximum: f64) -> Curve {
    let m = vertices.len();
    if maximum <= 0. {
        // Every segment is a corner: no alpha/angle tests or Bezier controls are needed.
        return (0..m)
            .map(|j| {
                let b = vertices[j];
                let end = vertices[(j + 1) % m].lerp(b, 0.5);
                Segment {
                    corner: true,
                    quadratic: false,
                    controls: [Point::default(), b, end],
                    vertex: b,
                    alpha: 0.,
                }
            })
            .collect();
    }
    let mut curve = vec![Segment::default(); m];
    for i in 0..m {
        let j = (i + 1) % m;
        let k = (i + 2) % m;
        let (a, b, c) = (vertices[i], vertices[j], vertices[k]);
        let end = c.lerp(b, 0.5);
        let denom = (c.x - a.x).abs() + (c.y - a.y).abs();
        let mut alpha = if denom != 0. {
            let d = (para(a, b, c) / denom).abs();
            if d > 1. {
                (1. - 1. / d) / 0.75
            } else {
                0.
            }
        } else {
            4. / 3.
        };
        let corner = alpha >= maximum;
        let controls = if corner {
            [Point::default(), b, end]
        } else {
            alpha = alpha.clamp(0.55, 1.);
            [
                a.lerp(b, 0.5 + 0.5 * alpha),
                c.lerp(b, 0.5 + 0.5 * alpha),
                end,
            ]
        };
        curve[j] = Segment {
            corner,
            quadratic: false,
            controls,
            vertex: b,
            alpha,
        };
    }
    curve
}
fn bezier(t: f64, p: [Point; 4]) -> Point {
    let s = 1. - t;
    let weights = [s * s * s, 3. * (s * s * t), 3. * (t * t * s), t * t * t];
    Point::new(
        (0..4).map(|i| p[i].x * weights[i]).sum(),
        (0..4).map(|i| p[i].y * weights[i]).sum(),
    )
}
fn tangent(p: [Point; 4], a: Point, b: Point) -> Option<f64> {
    let d = b.sub(a);
    let aa = p[1].sub(p[0]).cross(d);
    let bb = p[2].sub(p[1]).cross(d);
    let cc = p[3].sub(p[2]).cross(d);
    let (a, b, c) = (aa - 2. * bb + cc, -2. * aa + 2. * bb, aa);
    let det = b * b - 4. * a * c;
    if a == 0. || det < 0. {
        return None;
    }
    [(-b + det.sqrt()) / (2. * a), (-b - det.sqrt()) / (2. * a)]
        .into_iter()
        .find(|v| (0. ..=1.).contains(v))
}
#[derive(Clone, Default)]
struct Merge {
    penalty: f64,
    controls: [Point; 2],
    alpha: f64,
}
fn merge(
    curve: &Curve,
    i: usize,
    j: usize,
    tolerance: f64,
    convex: &[f64],
    areas: &[f64],
) -> Option<Merge> {
    let m = curve.len();
    if i == j {
        return None;
    }
    let i1 = (i + 1) % m;
    let conv = convex[i1];
    if conv == 0. {
        return None;
    }
    let base = curve[i1].vertex.sub(curve[i].vertex);
    let distance = (base.x * base.x + base.y * base.y).sqrt();
    let mut k = i1;
    while k != j {
        let k1 = (k + 1) % m;
        let k2 = (k + 2) % m;
        let edge = curve[k2].vertex.sub(curve[k1].vertex);
        if convex[k1] != conv
            || sign(base.cross(edge)) != conv
            || base.dot(edge)
                < distance * (edge.x * edge.x + edge.y * edge.y).sqrt() * -0.999847695156
        {
            return None;
        }
        k = k1;
    }
    let p0 = curve[i].controls[2];
    let p1 = curve[i1].vertex;
    let p2 = curve[j].vertex;
    let p3 = curve[j].controls[2];
    let area = areas[j] - areas[i] - para(curve[0].vertex, p0, p3) / 2.
        + if i >= j { areas[m] } else { 0. };
    let (a1, a2, a3) = (para(p0, p1, p2), para(p0, p1, p3), para(p0, p2, p3));
    let a4 = a1 + a3 - a2;
    if a2 == a1 || a3 == a4 {
        return None;
    }
    let t = a3 / (a3 - a4);
    let s = a2 / (a2 - a1);
    let a = a2 * t / 2.;
    if a == 0. {
        return None;
    }
    let radical = 4. - area / a / 0.3;
    if radical < 0. {
        return None;
    }
    let alpha = 2. - radical.sqrt();
    let controls = [p0.lerp(p1, t * alpha), p3.lerp(p2, s * alpha)];
    let p = [p0, controls[0], controls[1], p3];
    let mut penalty = 0.;
    let mut k = i1;
    while k != j {
        let k1 = (k + 1) % m;
        let (a, b) = (curve[k].vertex, curve[k1].vertex);
        let point = bezier(tangent(p, a, b)?, p);
        let distance = a.distance(b);
        if distance == 0. {
            return None;
        }
        let d = para(a, b, point) / distance;
        if d.abs() > tolerance || b.sub(a).dot(point.sub(a)) < 0. || a.sub(b).dot(point.sub(b)) < 0.
        {
            return None;
        }
        penalty += d * d;
        k = k1;
    }
    let mut k = i;
    while k != j {
        let k1 = (k + 1) % m;
        let (a, b) = (curve[k].controls[2], curve[k1].controls[2]);
        let point = bezier(tangent(p, a, b)?, p);
        let distance = a.distance(b);
        if distance == 0. {
            return None;
        }
        let mut d1 = para(a, b, point) / distance;
        let mut d2 = para(a, b, curve[k1].vertex) / distance * 0.75 * curve[k1].alpha;
        if d2 < 0. {
            d1 = -d1;
            d2 = -d2;
        }
        if d1 < d2 - tolerance {
            return None;
        }
        if d1 < d2 {
            penalty += (d1 - d2) * (d1 - d2);
        }
        k = k1;
    }
    Some(Merge {
        penalty,
        controls,
        alpha,
    })
}
fn optimize(curve: &Curve, tolerance: f64, control: &WorkControl) -> Curve {
    let m = curve.len();
    if m < 2 {
        return curve.clone();
    }
    let convex: Vec<f64> = (0..m)
        .map(|i| {
            if curve[i].corner {
                0.
            } else {
                sign(para(
                    curve[(i + m - 1) % m].vertex,
                    curve[i].vertex,
                    curve[(i + 1) % m].vertex,
                ))
            }
        })
        .collect();
    let mut areas = vec![0.; m + 1];
    let mut area = 0.;
    for i in 0..m {
        let j = (i + 1) % m;
        if !curve[j].corner {
            let alpha = curve[j].alpha;
            area += 0.3
                * alpha
                * (4. - alpha)
                * para(curve[i].controls[2], curve[j].vertex, curve[j].controls[2])
                / 2.;
            area += para(curve[0].vertex, curve[i].controls[2], curve[j].controls[2]) / 2.;
        }
        areas[i + 1] = area;
    }
    let mut previous = vec![0; m + 1];
    let mut penalties = vec![0.; m + 1];
    let mut lengths = vec![0; m + 1];
    let mut merges = vec![Merge::default(); m + 1];
    for j in 1..=m {
        previous[j] = j - 1;
        penalties[j] = penalties[j - 1];
        lengths[j] = lengths[j - 1] + 1;
        if j >= 2 {
            for i in (0..=j - 2).rev() {
                if i & 255 == 0 && control.cancelled() {
                    return Vec::new();
                }
                let Some(candidate) = merge(curve, i, j % m, tolerance, &convex, &areas) else {
                    break;
                };
                if lengths[j] > lengths[i] + 1
                    || (lengths[j] == lengths[i] + 1
                        && penalties[j] > penalties[i] + candidate.penalty)
                {
                    previous[j] = i;
                    penalties[j] = penalties[i] + candidate.penalty;
                    lengths[j] = lengths[i] + 1;
                    merges[j] = candidate;
                }
            }
        }
    }
    let mut result = Vec::new();
    let mut j = m;
    while j > 0 {
        if previous[j] == j - 1 {
            result.push(curve[j % m].clone());
        } else {
            let candidate = &merges[j];
            result.push(Segment {
                corner: false,
                quadratic: false,
                controls: [
                    candidate.controls[0],
                    candidate.controls[1],
                    curve[j % m].controls[2],
                ],
                vertex: curve[j % m].vertex,
                alpha: candidate.alpha,
            });
        }
        j = previous[j];
    }
    result.reverse();
    result
}

pub fn path_data(curves: &[Curve], height: usize) -> String {
    let mut result = String::new();
    for curve in curves {
        if curve.is_empty() {
            continue;
        }
        let start = curve.last().unwrap().controls[2];
        write!(result, "M{:.3} {:.3}", start.x, height as f64 - start.y).unwrap();
        for segment in curve {
            if segment.corner {
                for p in &segment.controls[1..] {
                    write!(result, "L{:.3} {:.3}", p.x, height as f64 - p.y).unwrap();
                }
            } else {
                result.push('C');
                for p in segment.controls {
                    write!(result, "{:.3} {:.3} ", p.x, height as f64 - p.y).unwrap();
                }
            }
        }
        result.push('Z');
    }
    result
}
