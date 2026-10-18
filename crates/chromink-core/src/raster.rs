//! Direct path rendering for desktop previews: SVG text never enters a browser parser.
use crate::{control::WorkControl, parallel::Parallelism, TraceDocument};
use tiny_skia::{FillRule, Paint, Path, PathBuilder, Pixmap, Transform};
pub struct Scene {
    pub width: u32,
    pub height: u32,
    paths: Vec<(
        Path,
        [u8; 4],
        Option<crate::Gradient>,
        f64,
        bool,
        bool,
        Option<Path>,
    )>,
}
fn coordinate(value: f64) -> f32 {
    value as f32
}
impl Scene {
    pub fn compile(document: &TraceDocument, control: &WorkControl) -> Result<Self, String> {
        let mut paths = Vec::new();
        for layer in &document.layers {
            control.check()?;
            let mut builder = PathBuilder::new();
            for (curve_index, curve) in layer.curves.iter().enumerate() {
                if curve_index & 255 == 0 {
                    control.check()?;
                }
                if curve.is_empty() {
                    continue;
                }
                let start = curve.last().unwrap().controls[2];
                let height = document.view_height as f64;
                builder.move_to(coordinate(start.x), coordinate(height - start.y));
                for segment in curve {
                    let p = segment.controls;
                    if segment.corner {
                        builder.line_to(coordinate(p[1].x), coordinate(height - p[1].y));
                        builder.line_to(coordinate(p[2].x), coordinate(height - p[2].y));
                    } else {
                        builder.cubic_to(
                            coordinate(p[0].x),
                            coordinate(height - p[0].y),
                            coordinate(p[1].x),
                            coordinate(height - p[1].y),
                            coordinate(p[2].x),
                            coordinate(height - p[2].y),
                        );
                    }
                }
                builder.close();
            }
            if let Some(path) = builder.finish() {
                let clip = layer.clip.as_ref().and_then(|curves| {
                    let mut builder = PathBuilder::new();
                    for curve in curves {
                        if curve.is_empty() {
                            continue;
                        }
                        let start = curve.last().unwrap().controls[2];
                        let height = document.view_height as f64;
                        builder.move_to(start.x as f32, (height - start.y) as f32);
                        for s in curve {
                            let p = s.controls;
                            if s.corner {
                                builder.line_to(p[1].x as f32, (height - p[1].y) as f32);
                                builder.line_to(p[2].x as f32, (height - p[2].y) as f32);
                            } else {
                                builder.cubic_to(
                                    p[0].x as f32,
                                    (height - p[0].y) as f32,
                                    p[1].x as f32,
                                    (height - p[1].y) as f32,
                                    p[2].x as f32,
                                    (height - p[2].y) as f32,
                                );
                            }
                        }
                        builder.close();
                    }
                    builder.finish()
                });
                paths.push((
                    path,
                    layer.color,
                    layer.gradient.clone(),
                    layer.stroke,
                    layer.evenodd,
                    layer.stroke_only,
                    clip,
                ));
            }
        }
        Ok(Self {
            width: document.view_width,
            height: document.view_height,
            paths,
        })
    }
    pub fn png(
        &self,
        width: u32,
        height: u32,
        x: f32,
        y: f32,
        stage_width: f32,
        stage_height: f32,
        workers: &Parallelism,
        control: &WorkControl,
    ) -> Result<Vec<u8>, String> {
        control.check()?;
        if width == 0
            || height == 0
            || u64::from(width) * u64::from(height) > 4_194_304
            || ![x, y, stage_width, stage_height]
                .iter()
                .all(|v| v.is_finite())
            || stage_width <= 0.
            || stage_height <= 0.
        {
            return Err("预览尺寸无效或超过 4MP 预算".into());
        }
        let _reservation = control.reserve(width as usize * height as usize * 12)?;
        let stripes = workers.active().min(4).min((height as usize / 128).max(1));
        let rows = (height as usize).div_ceil(stripes);
        let jobs: Vec<_> = (0..height as usize)
            .step_by(rows)
            .map(|start| (start, (height as usize - start).min(rows)))
            .collect();
        let pieces = workers.map(&jobs, |(start, count)| -> Result<(usize, Pixmap), String> {
            control.check()?;
            let mut tile = Pixmap::new(width, *count as u32).ok_or("无法分配预览内存")?;
            let transform = Transform::from_row(
                stage_width / self.width as f32,
                0.,
                0.,
                stage_height / self.height as f32,
                x,
                y - *start as f32,
            );
            for (path, color, gradient, stroke, evenodd, stroke_only, clip) in &self.paths {
                control.check()?;
                let mut paint = Paint::default();
                paint.set_color_rgba8(color[0], color[1], color[2], color[3]);
                if let Some(g) = gradient {
                    let stops: Vec<_> = g
                        .stops
                        .iter()
                        .map(|(offset, c)| {
                            tiny_skia::GradientStop::new(
                                *offset as f32,
                                tiny_skia::Color::from_rgba8(c[0], c[1], c[2], color[3]),
                            )
                        })
                        .collect();
                    let shader = if g.radial {
                        tiny_skia::RadialGradient::new(
                            tiny_skia::Point::from_xy(g.from[0] as f32, g.from[1] as f32),
                            tiny_skia::Point::from_xy(g.from[0] as f32, g.from[1] as f32),
                            g.to[0] as f32,
                            stops,
                            tiny_skia::SpreadMode::Pad,
                            Transform::identity(),
                        )
                    } else {
                        tiny_skia::LinearGradient::new(
                            tiny_skia::Point::from_xy(g.from[0] as f32, g.from[1] as f32),
                            tiny_skia::Point::from_xy(g.to[0] as f32, g.to[1] as f32),
                            stops,
                            tiny_skia::SpreadMode::Pad,
                            Transform::identity(),
                        )
                    };
                    if let Some(shader) = shader {
                        paint.shader = shader;
                    }
                }
                let mask = if let Some(clip) = clip {
                    let mut mask =
                        tiny_skia::Mask::new(width, *count as u32).ok_or("无法分配剪裁内存")?;
                    mask.fill_path(clip, FillRule::EvenOdd, true, transform);
                    Some(mask)
                } else {
                    None
                };
                if !stroke_only {
                    tile.fill_path(
                        path,
                        &paint,
                        if *evenodd {
                            FillRule::EvenOdd
                        } else {
                            FillRule::Winding
                        },
                        transform,
                        mask.as_ref(),
                    );
                }
                if *stroke > 0. {
                    let style = tiny_skia::Stroke {
                        width: *stroke as f32,
                        line_join: tiny_skia::LineJoin::Round,
                        ..Default::default()
                    };
                    tile.stroke_path(path, &paint, &style, transform, mask.as_ref());
                }
            }
            control.check()?;
            Ok((*start, tile))
        });
        let mut output = Pixmap::new(width, height).ok_or("无法分配预览内存")?;
        for result in pieces {
            let (start, tile) = result?;
            let offset = start * width as usize * 4;
            output.data_mut()[offset..offset + tile.data().len()].copy_from_slice(tile.data());
        }
        control.check()?;
        crate::preview_png::encode_pixmap(output)
    }
}
