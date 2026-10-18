//! Vecline's region/contour/Schneider pipeline, implemented in Rust.
//! MIT provenance: documentation/algorithms/VECLINE-LICENSE.txt.
mod alpha;
mod arcs;
pub(crate) mod emit;
mod fit;
mod geometry;
mod gradient;
mod math;
mod preprocess;
mod refine_source;
mod regions;
mod segment;
mod subpixel;
use crate::{
    control::WorkControl,
    outline::{Curve, Point, Segment},
    TraceLayer,
};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    pub vc_mode: String,
    pub vc_preset: String,
    pub vc_tolerance: f64,
    pub vc_fit_error: f64,
    pub vc_corner_angle: f64,
    pub vc_polygon_only: bool,
    pub vc_optimize: bool,
    pub vc_optimize_error: f64,
    pub vc_lattice_simplify: bool,
    pub vc_lattice_band: f64,
    pub vc_regularise: u8,
    pub vc_regularise_band: f64,
    pub vc_right_angle: bool,
    pub vc_right_angle_threshold: f64,
    pub vc_speckle_scope: String,
    pub vc_turn_policy: String,
    pub vc_min_area: u32,
    pub vc_alpha_levels: u8,
    pub vc_blur: u8,
    pub vc_blur_delta: u8,
    pub vc_precision: u8,
    pub vc_subpixel: bool,
    pub vc_smooth: f64,
    pub vc_segment: f64,
    pub vc_quadratics: bool,
    pub vc_mosaic: bool,
    pub vc_underpaint: bool,
    pub vc_interpolate: bool,
    pub vc_interpolate_width: f64,
    pub vc_extend_under: bool,
    pub vc_gradients: bool,
    pub vc_gradient_min_area: u32,
    pub vc_gradient_step_max: f64,
    pub vc_gradient_margin: f64,
    pub vc_gradient_max_error: f64,
    pub vc_gradient_stops: u8,
    pub vc_stroke_width: f64,
    pub vc_segment_min_region: u32,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            vc_mode: "trace".into(),
            vc_preset: "default".into(),
            vc_tolerance: 0.4,
            vc_fit_error: 0.4,
            vc_corner_angle: 75.,
            vc_polygon_only: false,
            vc_optimize: true,
            vc_optimize_error: 0.4,
            vc_lattice_simplify: true,
            vc_lattice_band: 0.75,
            vc_regularise: 0,
            vc_regularise_band: 0.75,
            vc_right_angle: false,
            vc_right_angle_threshold: 12.,
            vc_speckle_scope: "all".into(),
            vc_turn_policy: "left".into(),
            vc_min_area: 0,
            vc_alpha_levels: 8,
            vc_blur: 0,
            vc_blur_delta: 20,
            vc_precision: 1,
            vc_subpixel: true,
            vc_smooth: 0.,
            vc_quadratics: false,
            vc_mosaic: false,
            vc_underpaint: false,
            vc_interpolate: false,
            vc_interpolate_width: 1.,
            vc_extend_under: false,
            vc_gradients: false,
            vc_gradient_min_area: 0,
            vc_gradient_step_max: 0.08,
            vc_gradient_margin: 0.1,
            vc_gradient_max_error: 0.015,
            vc_gradient_stops: 16,
            vc_stroke_width: 0.,
            vc_segment: 0.,
            vc_segment_min_region: 0,
        }
    }
}
impl Options {
    /// Presets keep palette size and reducer selection in the shared workflow.
    pub fn set_preset(&mut self, name: &str) {
        let mode = self.vc_mode.clone();
        *self = Self::default();
        self.vc_mode = mode;
        self.vc_preset = name.into();
        match name {
            "logo" => {
                self.vc_min_area = 8;
                self.vc_tolerance = 0.65;
                self.vc_fit_error = 0.65;
            }
            "lineart" => {
                self.vc_min_area = 4;
                self.vc_tolerance = 0.6;
                self.vc_fit_error = 0.6;
                self.vc_corner_angle = 60.;
            }
            "poster" => {
                self.vc_min_area = 12;
                self.vc_tolerance = 0.5;
                self.vc_fit_error = 0.5;
                self.vc_corner_angle = 80.;
            }
            "photo" => {
                self.vc_gradients = true;
            }
            "detailed" => {
                self.vc_min_area = 2;
                self.vc_tolerance = 0.3;
                self.vc_fit_error = 0.3;
                self.vc_corner_angle = 60.;
            }
            "clean" => {
                self.vc_min_area = 4;
                self.vc_smooth = 1.;
                self.vc_segment = 0.02;
                self.vc_mosaic = true;
                self.vc_stroke_width = 0.5;
                self.vc_optimize_error = 0.75;
                self.vc_quadratics = true;
                self.vc_underpaint = true;
            }
            _ => {}
        }
    }
    pub(crate) fn resolved(&self, image: &image::RgbaImage) -> Self {
        if self.vc_preset == "default" {
            return self.clone();
        }
        let mut o = Self::default();
        o.set_preset(&self.vc_preset);
        o.vc_mode = self.vc_mode.clone();
        o.vc_tolerance = self.vc_tolerance;
        o.vc_fit_error = self.vc_fit_error;
        o.vc_min_area = self.vc_min_area;
        o.vc_optimize_error = self.vc_optimize_error;
        o.vc_gradients = self.vc_gradients;
        if !["logo", "lineart"].contains(&o.vc_preset.as_str()) && preprocess::noise(image) > 0.3 {
            o.vc_subpixel = false;
            o.vc_lattice_simplify = false;
        }
        if o.vc_preset == "clean"
            && image.width() as u64 * image.height() as u64 >= 2_000_000
            && o.vc_min_area == 4
        {
            o.vc_min_area = 40;
        }
        o
    }
    pub fn validate(&self) -> Result<(), String> {
        if !["trace", "auto", "pixel", "exact"].contains(&self.vc_mode.as_str()) {
            return Err("未知 Vecline 几何模式".into());
        }
        if ![
            "default", "logo", "lineart", "poster", "photo", "detailed", "clean",
        ]
        .contains(&self.vc_preset.as_str())
        {
            return Err("未知 Vecline 预设".into());
        }
        for (name, v, lo, hi) in [
            ("接缝宽度", self.vc_interpolate_width, 0.1, 10.),
            ("渐变颜色步长", self.vc_gradient_step_max, 0., 1.),
            ("渐变收益", self.vc_gradient_margin, 0., 1.),
            ("渐变误差", self.vc_gradient_max_error, 0., 1.),
            ("描边宽度", self.vc_stroke_width, 0., 10.),
            ("区域分割", self.vc_segment, 0., 10.),
            ("内部平滑", self.vc_smooth, 0., 1.),
            ("简化容差", self.vc_tolerance, 0., 10.),
            ("曲线误差", self.vc_fit_error, 0., 10.),
            ("合并误差", self.vc_optimize_error, 0., 10.),
            ("转角角度", self.vc_corner_angle, 0., 180.),
            ("格点带宽", self.vc_lattice_band, 0., 5.),
            ("边界带宽", self.vc_regularise_band, 0., 5.),
            ("直角偏差", self.vc_right_angle_threshold, 0., 45.),
        ] {
            if !v.is_finite() || v < lo || v > hi {
                return Err(format!("Vecline {name}无效"));
            }
        }
        if !(2..=32).contains(&self.vc_gradient_stops)
            || self.vc_gradient_min_area > 1000000
            || self.vc_segment_min_region > 100000
            || self.vc_regularise > 20
            || self.vc_alpha_levels == 0
            || self.vc_alpha_levels > 32
            || self.vc_blur > 5
            || self.vc_precision > 6
            || self.vc_min_area > 100000
            || !["all", "isolated"].contains(&self.vc_speckle_scope.as_str())
            || !["left", "right", "majority", "minority"].contains(&self.vc_turn_policy.as_str())
        {
            return Err("Vecline 参数无效".into());
        }
        Ok(())
    }
}
pub(crate) fn prepare(
    image: &image::RgbaImage,
    o: &Options,
    control: &WorkControl,
) -> Result<image::RgbaImage, String> {
    let alpha = alpha::collapse(image, o.vc_alpha_levels, control)?;
    let blurred = preprocess::selective_blur(&alpha, o.vc_blur, o.vc_blur_delta, control)?;
    let smooth = preprocess::smooth(&blurred, o.vc_smooth, control)?;
    segment::flatten(
        &smooth,
        o.vc_segment,
        o.vc_segment_min_region as usize,
        control,
    )
}
pub(crate) fn trace(
    palette: &[[u8; 4]],
    ids: &[u16],
    image: &image::RgbaImage,
    o: &Options,
    mono: bool,
    control: &WorkControl,
) -> Result<Vec<TraceLayer>, String> {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let mode = geometry::mode(image, o, control)?;
    if mode != "trace" {
        return geometry::trace(palette, ids, image, &mode, control);
    }
    // Resolve incompatible stages once for CLI, restored sessions and the GUI.
    let mut effective = o.clone();
    effective.vc_gradients &= !mono && !effective.vc_extend_under;
    if effective.vc_extend_under {
        effective.vc_subpixel = false;
        effective.vc_mosaic = false;
        effective.vc_regularise = 0;
        effective.vc_interpolate = false;
        effective.vc_underpaint = false;
        effective.vc_lattice_simplify = false;
    }
    if effective.vc_mosaic || effective.vc_regularise > 0 {
        effective.vc_right_angle = false;
    }
    if effective.vc_interpolate {
        effective.vc_stroke_width = 0.;
    }
    let o = &effective;
    let (expanded, mapped) = if mono {
        (palette.to_vec(), ids.to_vec())
    } else {
        alpha::expand(palette, ids, image, o.vc_alpha_levels, control)?
    };
    let (mut palette, ids) = (expanded, &mapped);
    let mut classes: Vec<_> = ids
        .iter()
        .map(|&p| {
            if p == crate::TRANSPARENT {
                -1
            } else {
                p as i32
            }
        })
        .collect();
    let paints = gradient::detect(image, &mut classes, &mut palette, o, control)?;
    for _ in 0..2 {
        regions::absorb(
            &mut classes,
            w,
            h,
            o.vc_min_area as usize,
            &o.vc_speckle_scope,
            control,
        )?;
    }
    let measurement = if o.vc_subpixel || o.vc_mosaic || o.vc_regularise > 0 {
        std::borrow::Cow::Owned(refine_source::run(image, &classes, control)?)
    } else {
        std::borrow::Cow::Borrowed(image)
    };
    let mut loops = regions::contours(&classes, w, h, palette.len(), &o.vc_turn_policy, control)?;
    let mut area = vec![0usize; palette.len()];
    for &cls in &classes {
        if cls >= 0 {
            area[cls as usize] += 1;
        }
    }
    let mut order: Vec<_> = (0..palette.len()).collect();
    order.sort_by(|&a, &b| area[b].cmp(&area[a]).then_with(|| a.cmp(&b)));
    let extend = o.vc_extend_under && palette.iter().all(|p| p[3] == 255) && !o.vc_gradients;
    if extend {
        let mut ranks = vec![0usize; palette.len()];
        for (rank, &id) in order.iter().enumerate() {
            ranks[id] = rank;
        }
        for (rank, &id) in order.iter().enumerate() {
            control.check()?;
            let union: Vec<_> = classes
                .iter()
                .map(|&cls| {
                    if cls >= 0 && ranks[cls as usize] >= rank {
                        0
                    } else {
                        -1
                    }
                })
                .collect();
            loops[id] = regions::contours(&union, w, h, 1, &o.vc_turn_policy, control)?
                .pop()
                .unwrap();
        }
    }
    let regularised = !extend && !o.vc_mosaic && o.vc_regularise > 0;
    if regularised {
        loops = arcs::regularise(&loops, &classes, w, h, o, control)?;
    }
    let shared = if !extend && o.vc_mosaic {
        Some(arcs::fit(&loops, &classes, &measurement, o, control)?)
    } else {
        None
    };
    let has_void = classes.iter().any(|&v| v < 0);
    let tight_under = o.vc_underpaint && has_void && !extend;
    let mut under_options = o.clone();
    under_options.vc_optimize_error = o.vc_fit_error;
    let under_shared = if tight_under && shared.is_some() {
        Some(arcs::fit(
            &loops,
            &classes,
            &measurement,
            &under_options,
            control,
        )?)
    } else {
        None
    };
    let fitted_layers = loops
        .into_par_iter()
        .enumerate()
        .map(|(i, loops)| {
            control.check()?;
            let mut curves = Vec::new();
            let mut shapes = Vec::new();
            let mut under_curves = Vec::new();
            for (loop_index, points) in loops.into_iter().enumerate() {
                let refined = if shared.is_none() && !extend && !regularised && o.vc_subpixel {
                    subpixel::refine(&points, &measurement, &classes, i as i32, control)?
                } else {
                    points
                };
                let mut fit_options = o.clone();
                fit_options.vc_regularise = 0;
                if o.vc_subpixel || extend || regularised {
                    fit_options.vc_lattice_simplify = false;
                }
                let fitted = if let Some(shared) = &shared {
                    shared[i][loop_index].clone()
                } else {
                    fit::closed(&refined, &fit_options, control)?
                };
                if tight_under && palette[i][3] == 255 {
                    let tight = if let Some(ref under_shared) = under_shared {
                        under_shared[i][loop_index].clone()
                    } else {
                        let mut under = fit_options.clone();
                        under.vc_optimize_error = o.vc_fit_error;
                        fit::closed(&refined, &under, control)?
                    };
                    if !tight.is_empty() {
                        control.claim(tight.len() * std::mem::size_of::<Segment>())?;
                        under_curves.push(scene_curve(tight, h));
                    }
                }
                let shape = None;
                if fitted.is_empty() {
                    continue;
                }
                control.claim(fitted.len() * std::mem::size_of::<Segment>())?;
                let curve = scene_curve(fitted, h);
                curves.push(curve);
                shapes.push(shape);
            }
            Ok((
                TraceLayer {
                    color: palette[i],
                    curves,
                    gradient: paints[i].clone(),
                    stroke: o.vc_stroke_width,
                    evenodd: true,
                    stroke_only: false,
                    clip: None,
                    svg_shapes: shapes,
                },
                under_curves,
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let (layers, under_curves): (Vec<_>, Vec<_>) = fitted_layers.into_iter().unzip();
    let mut ordered: Vec<_> = order.iter().map(|&i| layers[i].clone()).collect();
    let opaque = classes.iter().all(|&v| v >= 0) && palette.iter().all(|p| p[3] == 255);
    if opaque && ordered.len() > 1 && ordered[0].gradient.is_none() {
        let top = &mut ordered[0];
        top.svg_shapes.clear();
        top.curves = vec![vec![
            Segment::from_controls(
                true,
                [
                    Point { x: 0., y: h as f64 },
                    Point {
                        x: w as f64,
                        y: h as f64,
                    },
                    Point {
                        x: w as f64,
                        y: h as f64,
                    },
                ],
            ),
            Segment::from_controls(
                true,
                [
                    Point {
                        x: w as f64,
                        y: h as f64,
                    },
                    Point { x: w as f64, y: 0. },
                    Point { x: w as f64, y: 0. },
                ],
            ),
            Segment::from_controls(
                true,
                [
                    Point { x: w as f64, y: 0. },
                    Point { x: 0., y: 0. },
                    Point { x: 0., y: 0. },
                ],
            ),
            Segment::from_controls(
                true,
                [
                    Point { x: 0., y: 0. },
                    Point { x: 0., y: h as f64 },
                    Point { x: 0., y: h as f64 },
                ],
            ),
        ]];
    }
    let mut backing = Vec::new();
    if o.vc_underpaint && !opaque && !extend {
        let curves = order
            .iter()
            .flat_map(|&i| under_curves[i].clone())
            .collect::<Vec<_>>();
        if !curves.is_empty() {
            let mut layer = ordered.iter().find(|l| l.color[3] == 255).unwrap().clone();
            layer.curves = curves;
            layer.gradient = None;
            layer.svg_shapes.clear();
            layer.stroke = 0.;
            backing.push(layer);
        }
    }
    if o.vc_interpolate && !extend {
        let mut adj = std::collections::BTreeMap::<(usize, usize), usize>::new();
        for y in 0..h {
            control.check()?;
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
                    if b >= 0 && b != a {
                        let (a, b) = (a as usize, b as usize);
                        *adj.entry((a.min(b), a.max(b))).or_default() += 1;
                    }
                }
            }
        }
        for ((a, b), len) in adj {
            if len < 8 || layers[a].gradient.is_some() || layers[b].gradient.is_some() {
                continue;
            }
            let (ca, cb) = (palette[a], palette[b]);
            let alpha = ca[3] as f64 + cb[3] as f64;
            if alpha == 0. {
                continue;
            }
            let rgb: [u8; 3] = std::array::from_fn(|i| {
                ((ca[i] as f64 * ca[3] as f64 + cb[i] as f64 * cb[3] as f64) / alpha).round() as u8
            });
            let color = [rgb[0], rgb[1], rgb[2], (alpha / 2.).round() as u8];
            for (from, to) in [(a, b), (b, a)] {
                let mut layer = layers[from].clone();
                layer.color = color;
                layer.stroke_only = true;
                layer.svg_shapes.clear();
                layer.stroke = o.vc_interpolate_width;
                layer.clip = Some(layers[to].curves.clone());
                backing.push(layer);
            }
        }
    }
    backing.append(&mut ordered);
    Ok(backing)
}

fn scene_curve(fitted: Vec<fit::S>, height: usize) -> Curve {
    fitted
        .into_iter()
        .map(|s| {
            let mut segment = Segment::from_controls(
                s.line,
                s.points.map(|p| Point {
                    x: p.x,
                    y: height as f64 - p.y,
                }),
            );
            segment.quadratic = s.quad;
            segment
        })
        .collect()
}
