//! Coverage projection, plateau search and corner-gated displacement median.
//! Vecline subpixel.ts, MIT. All loops use image coordinates (y downward).
use super::fit::P;
use crate::control::WorkControl;
pub(super) fn refine(
    input: &[P],
    image: &image::RgbaImage,
    classes: &[i32],
    class: i32,
    c: &WorkControl,
) -> Result<Vec<P>, String> {
    measure(input, image, classes, class, c, true)
}
pub(super) fn open(
    input: &[P],
    image: &image::RgbaImage,
    classes: &[i32],
    class: i32,
    c: &WorkControl,
) -> Result<Vec<P>, String> {
    measure(input, image, classes, class, c, false)
}
fn measure(
    input: &[P],
    image: &image::RgbaImage,
    classes: &[i32],
    class: i32,
    c: &WorkControl,
    closed: bool,
) -> Result<Vec<P>, String> {
    let count: usize = (0..if closed {
        input.len()
    } else {
        input.len().saturating_sub(1)
    })
        .map(|i| {
            let a = input[i];
            let b = input[(i + 1) % input.len()];
            ((b.x - a.x).abs() + (b.y - a.y).abs()) as usize
        })
        .sum::<usize>()
        + usize::from(!closed);
    let _scratch = c.reserve(count * 96)?;
    let (w, h) = (image.width() as isize, image.height() as isize);
    let mut points = Vec::with_capacity(count);
    for i in 0..if closed { input.len() } else { input.len() - 1 } {
        let a = input[i];
        let b = input[(i + 1) % input.len()];
        let (dx, dy) = (
            if b.x == a.x { 0. } else { (b.x - a.x).signum() },
            if b.y == a.y { 0. } else { (b.y - a.y).signum() },
        );
        let n = ((b.x - a.x).abs() + (b.y - a.y).abs()) as usize;
        for step in 0..n {
            points.push(P::new(a.x + dx * step as f64, a.y + dy * step as f64));
        }
    }
    if !closed {
        points.push(*input.last().unwrap());
    }
    let n = points.len();
    let edges = if closed { n } else { n - 1 };
    if n < 3 {
        return Ok(input.to_vec());
    }

    let directions: Vec<_> = (0..edges)
        .map(|i| {
            (
                points[(i + 1) % n].x - points[i].x,
                points[(i + 1) % n].y - points[i].y,
            )
        })
        .collect();
    let mut runs = vec![0usize; n];
    let mut lengths = Vec::new();
    let mut start = 0;
    for i in 1..=edges {
        if i == edges || directions[i] != directions[i - 1] {
            let id = lengths.len();
            for v in &mut runs[start..i] {
                *v = id;
            }
            lengths.push(i - start);
            start = i;
        }
    }
    if closed && lengths.len() > 1 && directions[0] == directions[edges - 1] {
        let id = runs[n - 1];
        lengths[0] += lengths[id];
        for v in &mut runs {
            if *v == id {
                *v = 0;
            }
        }
    }
    let cuts: Vec<_> = (0..n)
        .map(|i| {
            if !closed && (i == 0 || i == n - 1) {
                true
            } else {
                runs[(i + n - 1) % n] != runs[i]
                    && lengths[runs[(i + n - 1) % n]] >= 4
                    && lengths[runs[i]] >= 4
            }
        })
        .collect();
    let mut sum = vec![[0.; 2]; n];
    let mut hits = vec![[0u8; 2]; n];
    let mut saturated = vec![[false; 2]; n];
    let inside = |x: isize, y: isize| x >= 0 && y >= 0 && x < w && y < h;
    let at = |x: isize, y: isize| (y * w + x) as usize;
    let plateau = |x0: isize, y0: isize, dx: isize, dy: isize, cls: i32| {
        let mut previous = at(x0, y0);
        for step in 1..=12 {
            let (x, y) = (x0 + dx * step, y0 + dy * step);
            if !inside(x, y) || classes[at(x, y)] != cls {
                break;
            }
            let next = at(x, y);
            let a = &image.as_raw()[previous * 4..previous * 4 + 4];
            let b = &image.as_raw()[next * 4..next * 4 + 4];
            let distance: i32 = (0..4)
                .map(|ch| {
                    let d = a[ch] as i32 - b[ch] as i32;
                    d * d
                })
                .sum();
            previous = next;
            if distance <= 16 {
                break;
            }
        }
        previous
    };
    for i in 0..edges {
        if i & 8191 == 0 {
            c.check()?;
        }
        let j = (i + 1) % n;
        let (a, b) = (points[i], points[j]);
        let (x1, y1, x2, y2) = if a.y == b.y {
            (
                a.x.min(b.x) as isize,
                a.y as isize - 1,
                a.x.min(b.x) as isize,
                a.y as isize,
            )
        } else {
            (
                a.x as isize - 1,
                a.y.min(b.y) as isize,
                a.x as isize,
                a.y.min(b.y) as isize,
            )
        };
        let in1 = inside(x1, y1) && classes[at(x1, y1)] == class;
        let in2 = inside(x2, y2) && classes[at(x2, y2)] == class;
        if in1 == in2 {
            continue;
        }
        let (ix, iy, ox, oy) = if in1 {
            (x1, y1, x2, y2)
        } else {
            (x2, y2, x1, y1)
        };
        if !inside(ox, oy) {
            continue;
        }
        let (nx, ny) = (ox - ix, oy - iy);
        let inner = plateau(ix, iy, -nx, -ny, class);
        let outer = plateau(ox, oy, nx, ny, classes[at(ox, oy)]);
        let data = image.as_raw();
        let delta: [f64; 4] =
            std::array::from_fn(|ch| data[inner * 4 + ch] as f64 - data[outer * 4 + ch] as f64);
        let contrast: f64 = delta.iter().map(|v| v * v).sum();
        if contrast < 36. {
            continue;
        }
        let project = |index: usize| {
            ((0..4)
                .map(|ch| (data[index * 4 + ch] as f64 - data[outer * 4 + ch] as f64) * delta[ch])
                .sum::<f64>()
                / contrast)
                .clamp(0., 1.)
        };
        let raw = project(at(ix, iy)) + project(at(ox, oy)) - 1.;
        let shift = raw.clamp(-0.5, 0.5);
        if shift == 0. {
            continue;
        }
        let (axis, normal) = if nx != 0 {
            (0, nx as f64)
        } else {
            (1, ny as f64)
        };
        for vertex in [i, j] {
            sum[vertex][axis] += shift * normal;
            hits[vertex][axis] += 1;
            saturated[vertex][axis] |= raw.abs() >= 0.5;
        }
    }
    let valid: Vec<_> = hits
        .iter()
        .enumerate()
        .map(|(i, v)| (closed || i > 0 && i + 1 < n) && (v[0] > 0 || v[1] > 0))
        .collect();
    let offsets: Vec<_> = (0..n)
        .map(|i| {
            let (x, y) = (hits[i][0], hits[i][1]);
            if x == 0 && y == 0 {
                return [0.; 2];
            }
            if y == 0 {
                return [sum[i][0] / x as f64, 0.];
            }
            if x == 0 {
                return [0., sum[i][1] / y as f64];
            }
            if saturated[i][0] && !saturated[i][1] {
                return [0., sum[i][1] / y as f64];
            }
            if saturated[i][1] && !saturated[i][0] {
                return [sum[i][0] / x as f64, 0.];
            }
            [sum[i][0] / (x + y) as f64, sum[i][1] / (x + y) as f64]
        })
        .collect();
    let mut result = points.clone();
    for i in 0..n {
        if !valid[i] {
            continue;
        }
        let mut window = Vec::new();
        for r in (1..=2).rev() {
            if !closed && r > i {
                continue;
            }
            let j = (i + n - r % n) % n;
            if !(0..r).any(|b| cuts[(i + n - b % n) % n]) && valid[j] {
                window.push(offsets[j]);
            }
        }
        window.push(offsets[i]);
        for r in 1..=2 {
            if !closed && i + r >= n {
                continue;
            }
            let j = (i + r) % n;
            if !(1..=r).any(|b| cuts[(i + b) % n]) && valid[j] {
                window.push(offsets[j]);
            }
        }
        let oscillation = window.windows(2).any(|v| {
            v[0][0].abs() + v[0][1].abs() >= 0.25
                && v[1][0].abs() + v[1][1].abs() >= 0.25
                && v[0][0] * v[1][0] + v[0][1] * v[1][1] < 0.
        });
        let mut offset = offsets[i];
        if oscillation {
            for axis in 0..2 {
                let mut values: Vec<_> = window.iter().map(|v| v[axis]).collect();
                values.sort_by(f64::total_cmp);
                offset[axis] = if values.len() % 2 == 1 {
                    values[values.len() / 2]
                } else {
                    (values[values.len() / 2 - 1] + values[values.len() / 2]) / 2.
                };
            }
        }
        result[i] = P::new(points[i].x + offset[0], points[i].y + offset[1]);
    }
    Ok(result)
}
