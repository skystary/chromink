//! Connected components, longest-border speck absorption, crack following.
use super::fit::P;
use crate::control::WorkControl;
struct Component {
    class: i32,
    area: usize,
    bounds: [usize; 4],
}
fn label(
    classes: &[i32],
    w: usize,
    h: usize,
    c: &WorkControl,
) -> Result<(Vec<i32>, Vec<Component>), String> {
    let mut labels = vec![-1; classes.len()];
    let mut components = Vec::new();
    let mut queue = Vec::new();
    for start in 0..classes.len() {
        if start & 8191 == 0 {
            c.check()?;
        }
        if classes[start] < 0 || labels[start] >= 0 {
            continue;
        }
        let id = components.len() as i32;
        let class = classes[start];
        queue.clear();
        queue.push(start);
        labels[start] = id;
        let mut comp = Component {
            class,
            area: 0,
            bounds: [w, h, 0, 0],
        };
        let mut cursor = 0;
        while cursor < queue.len() {
            if cursor & 8191 == 0 {
                c.check()?;
            }
            let i = queue[cursor];
            cursor += 1;
            let (x, y) = (i % w, i / w);
            comp.area += 1;
            comp.bounds[0] = comp.bounds[0].min(x);
            comp.bounds[1] = comp.bounds[1].min(y);
            comp.bounds[2] = comp.bounds[2].max(x);
            comp.bounds[3] = comp.bounds[3].max(y);
            for j in neighbors(i, w, h) {
                if labels[j] < 0 && classes[j] == class {
                    labels[j] = id;
                    queue.push(j);
                }
            }
        }
        components.push(comp);
    }
    Ok((labels, components))
}
fn neighbors(i: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let (x, y) = (i % w, i / w);
    [
        if x > 0 { Some(i - 1) } else { None },
        if x + 1 < w { Some(i + 1) } else { None },
        if y > 0 { Some(i - w) } else { None },
        if y + 1 < h { Some(i + w) } else { None },
    ]
    .into_iter()
    .flatten()
}
pub(super) fn absorb(
    classes: &mut [i32],
    w: usize,
    h: usize,
    min_area: usize,
    scope: &str,
    c: &WorkControl,
) -> Result<(), String> {
    if min_area <= 1 {
        return Ok(());
    }
    let (labels, comps) = label(classes, w, h, c)?;
    let mut small: Vec<_> = comps
        .iter()
        .enumerate()
        .filter(|(_, v)| v.area < min_area)
        .map(|(i, _)| i)
        .collect();
    if scope == "isolated" {
        small.retain(|&id| {
            let comp = &comps[id];
            let mut other = None;
            for y in comp.bounds[1]..=comp.bounds[3] {
                for x in comp.bounds[0]..=comp.bounds[2] {
                    let i = y * w + x;
                    if labels[i] != id as i32 {
                        continue;
                    }
                    for j in neighbors(i, w, h) {
                        if classes[j] == comp.class {
                            continue;
                        }
                        if other.is_some_and(|v| v != classes[j]) {
                            return false;
                        }
                        other = Some(classes[j]);
                    }
                }
            }
            true
        });
    }
    small.sort_by_key(|&i| comps[i].area);
    let mut is_small = vec![false; comps.len()];
    for &i in &small {
        is_small[i] = true;
    }
    for id in small {
        c.check()?;
        let comp = &comps[id];
        let mut tally: Vec<(i32, usize)> = Vec::new();
        for y in comp.bounds[1]..=comp.bounds[3] {
            for x in comp.bounds[0]..=comp.bounds[2] {
                let i = y * w + x;
                if labels[i] != id as i32 {
                    continue;
                }
                for j in neighbors(i, w, h) {
                    let other = labels[j];
                    if other == id as i32 || other >= 0 && is_small[other as usize] {
                        continue;
                    }
                    let class = classes[j];
                    if let Some(v) = tally.iter_mut().find(|v| v.0 == class) {
                        v.1 += 1;
                    } else {
                        tally.push((class, 1));
                    }
                }
            }
        }
        let Some(&(class, _)) = tally
            .iter()
            .enumerate()
            .max_by(|(i, a), (j, b)| a.1.cmp(&b.1).then_with(|| j.cmp(i)))
            .map(|(_, v)| v)
        else {
            continue;
        };
        for y in comp.bounds[1]..=comp.bounds[3] {
            for x in comp.bounds[0]..=comp.bounds[2] {
                let i = y * w + x;
                if labels[i] == id as i32 {
                    classes[i] = class;
                }
            }
        }
        is_small[id] = false;
    }
    Ok(())
}
struct Edge {
    from: usize,
    to: usize,
    comp: i32,
    next: i32,
    used: bool,
}
pub(super) fn contours(
    classes: &[i32],
    w: usize,
    h: usize,
    class_count: usize,
    policy: &str,
    c: &WorkControl,
) -> Result<Vec<Vec<Vec<P>>>, String> {
    let (labels, comps) = label(classes, w, h, c)?;
    let stride = w + 1;
    let mut count = 0;
    for i in 0..labels.len() {
        if labels[i] < 0 {
            continue;
        }
        let (x, y) = (i % w, i / w);
        count += usize::from(y == 0 || labels[i - w] != labels[i])
            + usize::from(x + 1 == w || labels[i + 1] != labels[i])
            + usize::from(y + 1 == h || labels[i + w] != labels[i])
            + usize::from(x == 0 || labels[i - 1] != labels[i]);
    }
    let _scratch = c.reserve(count * std::mem::size_of::<Edge>() + stride * (h + 1) * 4)?;
    let mut head = vec![-1; stride * (h + 1)];
    let mut edges = Vec::with_capacity(count);
    let mut emit = |from: usize, to: usize, comp: i32| {
        edges.push(Edge {
            from,
            to,
            comp,
            next: head[from],
            used: false,
        });
        head[from] = edges.len() as i32 - 1;
    };
    for y in 0..h {
        c.check()?;
        for x in 0..w {
            let i = y * w + x;
            let comp = labels[i];
            if comp < 0 {
                continue;
            }
            let a = y * stride + x;
            let b = a + 1;
            let d = a + stride;
            let e = d + 1;
            if y == 0 || labels[i - w] != comp {
                emit(a, b, comp);
            }
            if x + 1 == w || labels[i + 1] != comp {
                emit(b, e, comp);
            }
            if y + 1 == h || labels[i + w] != comp {
                emit(e, d, comp);
            }
            if x == 0 || labels[i - 1] != comp {
                emit(d, a, comp);
            }
        }
    }
    let mut result = vec![Vec::new(); class_count];
    for start in 0..edges.len() {
        if start & 8191 == 0 {
            c.check()?;
        }
        if edges[start].used {
            continue;
        }
        let comp = edges[start].comp;
        let mut points = Vec::new();
        let mut edge = start;
        loop {
            if points.len() & 8191 == 0 {
                c.check()?;
            }
            let e = &edges[edge];
            let (from, to) = (e.from, e.to);
            points.push(P::new((from % stride) as f64, (from / stride) as f64));
            edges[edge].used = true;
            if to == edges[start].from {
                break;
            }
            let dx = (to % stride) as isize - (from % stride) as isize;
            let dy = (to / stride) as isize - (from / stride) as isize;
            let dirs = [(-dy, dx), (dx, dy), (dy, -dx), (-dx, -dy)];
            let mut choices = [None; 4];
            let (vx, vy) = ((to % stride) as isize, (to / stride) as isize);
            for (d, (mx, my)) in dirs.iter().enumerate() {
                let (nx, ny) = (vx + mx, vy + my);
                if nx < 0 || ny < 0 || nx > w as isize || ny > h as isize {
                    continue;
                }
                let target = ny as usize * stride + nx as usize;
                let mut j = head[to];
                while j >= 0 {
                    let e = &edges[j as usize];
                    if !e.used && e.comp == comp && e.to == target {
                        choices[d] = Some(j as usize);
                        break;
                    }
                    j = e.next;
                }
            }
            let hug = match policy {
                "right" => true,
                "majority" => majority(vx, vy, comp, &labels, w, h),
                "minority" => !majority(vx, vy, comp, &labels, w, h),
                _ => false,
            };
            let next = if choices[0].is_some() && choices[2].is_some() {
                choices[if hug { 2 } else { 0 }]
            } else {
                choices.into_iter().flatten().next()
            };
            let Some(next) = next else {
                return Err("Vecline 轮廓没有闭合".into());
            };
            edge = next;
        }
        let n = points.len();
        let mut collapsed = Vec::new();
        for i in 0..n {
            let a = points[i] - points[(i + n - 1) % n];
            let b = points[(i + 1) % n] - points[i];
            if a.cross(b) != 0. {
                collapsed.push(points[i]);
            }
        }
        if collapsed.len() >= 3 {
            c.claim(collapsed.len() * std::mem::size_of::<P>())?;
            result[comps[comp as usize].class as usize].push(collapsed);
        }
    }
    Ok(result)
}
fn majority(x: isize, y: isize, comp: i32, labels: &[i32], w: usize, h: usize) -> bool {
    let value = |x: isize, y: isize| {
        if x >= 0
            && y >= 0
            && x < w as isize
            && y < h as isize
            && labels[y as usize * w + x as usize] == comp
        {
            1
        } else {
            -1
        }
    };
    for radius in 2..=4 {
        let mut total = 0;
        for i in -radius..radius {
            total += value(x + i, y - radius)
                + value(x + radius - 1, y + i)
                + value(x + i, y + radius - 1)
                + value(x - radius, y + i);
        }
        if total > 0 {
            return true;
        }
        if total < 0 {
            return false;
        }
    }
    false
}
