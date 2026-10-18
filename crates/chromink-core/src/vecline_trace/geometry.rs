//! Integer geometry of the reduced bitmap, with no embedded raster payload.
use super::{fit::P, regions, Options};
use crate::{
    control::WorkControl,
    outline::{Curve, Point, Segment},
    TraceLayer, TRANSPARENT,
};
use std::collections::{HashMap, HashSet};

pub(super) fn mode(
    image: &image::RgbaImage,
    o: &Options,
    c: &WorkControl,
) -> Result<String, String> {
    if o.vc_mode != "auto" {
        return Ok(o.vc_mode.clone());
    }
    // Explicit content presets select the curve pipeline, as in upstream.
    if o.vc_preset != "default" {
        return Ok("trace".into());
    }
    let mut colors = HashSet::new();
    let mut runs = 0usize;
    let mut previous = None;
    for (i, p) in image.pixels().enumerate() {
        if i & 8191 == 0 {
            c.check()?;
        }
        let key = if p[3] == 0 { [0; 4] } else { p.0 };
        colors.insert(key);
        if colors.len() > 4096 {
            return Ok("trace".into());
        }
        if i % image.width() as usize == 0 || previous != Some(key) {
            runs += 1;
        }
        previous = Some(key);
    }
    let flat = runs <= 600_000
        && (runs as f64 / image.len().saturating_div(4).max(1) as f64 <= 0.12
            || colors.len() <= 64);
    Ok(if flat { "pixel" } else { "trace" }.into())
}
fn polygon(points: &[P], height: usize) -> Curve {
    let p = |v: P| Point {
        x: v.x,
        y: height as f64 - v.y,
    };
    (0..points.len())
        .map(|i| {
            let a = p(points[i]);
            let b = p(points[(i + 1) % points.len()]);
            Segment::from_controls(true, [a, a, b])
        })
        .collect()
}
pub(super) fn trace(
    palette: &[[u8; 4]],
    ids: &[u16],
    image: &image::RgbaImage,
    mode: &str,
    c: &WorkControl,
) -> Result<Vec<TraceLayer>, String> {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let _scratch = c.reserve(ids.len().saturating_mul(16))?;
    let mut expanded = Vec::new();
    let mut lookup = HashMap::new();
    let mut mapped = Vec::with_capacity(ids.len());
    for (i, (&id, p)) in ids.iter().zip(image.pixels()).enumerate() {
        if i & 8191 == 0 {
            c.check()?;
        }
        if id == TRANSPARENT || p[3] == 0 {
            mapped.push(TRANSPARENT);
            continue;
        }
        let cls = *lookup.entry((id, p[3])).or_insert_with(|| {
            let cls = expanded.len() as u16;
            let rgb = palette[id as usize];
            expanded.push([rgb[0], rgb[1], rgb[2], p[3]]);
            cls
        });
        mapped.push(cls);
    }
    let ids = mapped;
    let palette = &expanded;
    let mut curves: Vec<Vec<Curve>> = vec![Vec::new(); palette.len()];
    if mode == "exact" {
        let classes: Vec<_> = ids
            .iter()
            .map(|&i| if i == TRANSPARENT { -1 } else { i as i32 })
            .collect();
        for (i, loops) in regions::contours(&classes, w, h, palette.len(), "left", c)?
            .into_iter()
            .enumerate()
        {
            for points in loops {
                c.claim(points.len().saturating_mul(std::mem::size_of::<Segment>()))?;
                curves[i].push(polygon(&points, h));
            }
        }
    } else {
        // Merge identical horizontal runs vertically; map lookups do not affect output order.
        let mut rectangles: Vec<(usize, usize, usize, usize, usize)> = Vec::new();
        let mut prior = HashMap::new();
        for y in 0..h {
            c.check()?;
            let mut next = HashMap::new();
            let mut x = 0;
            while x < w {
                let start = x;
                let id = ids[y * w + x];
                x += 1;
                while x < w && ids[y * w + x] == id {
                    x += 1;
                }
                if id == TRANSPARENT {
                    continue;
                }
                let key = (id, start, x);
                let index = if let Some(&i) = prior.get(&key) {
                    let i: usize = i;
                    rectangles[i].4 += 1;
                    i
                } else {
                    c.claim(
                        std::mem::size_of::<(usize, usize, usize, usize, usize)>()
                            + 4 * std::mem::size_of::<Segment>(),
                    )?;
                    let i = rectangles.len();
                    rectangles.push((id as usize, start, y, x - start, 1));
                    i
                };
                next.insert(key, index);
            }
            prior = next;
        }
        for (id, x, y, rw, rh) in rectangles {
            curves[id].push(polygon(
                &[
                    P::new(x as f64, y as f64),
                    P::new((x + rw) as f64, y as f64),
                    P::new((x + rw) as f64, (y + rh) as f64),
                    P::new(x as f64, (y + rh) as f64),
                ],
                h,
            ));
        }
    }
    Ok(curves
        .into_iter()
        .enumerate()
        .filter(|(_, v)| !v.is_empty())
        .map(|(i, curves)| TraceLayer {
            color: palette[i],
            curves,
            gradient: None,
            stroke: 0.,
            evenodd: true,
            stroke_only: false,
            clip: None,
            svg_shapes: Vec::new(),
        })
        .collect())
}
