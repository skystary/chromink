//! Compact SVG geometry uses the same rounded scene as the direct preview.
use crate::{outline::Curve, TraceLayer};
use std::fmt::Write;
fn number(v: f64, precision: u8) -> String {
    let scale = 10f64.powi(precision as i32);
    let v = (v * scale).round() / scale;
    let mut s = format!("{v:.p$}", p = precision as usize);
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s = "0".into();
    }
    s
}
pub(crate) fn path(curves: &[Curve], height: usize, precision: u8) -> String {
    let mut text = String::new();
    let p = |v: crate::outline::Point| {
        format!(
            "{} {}",
            number(v.x, precision),
            number(height as f64 - v.y, precision)
        )
    };
    for curve in curves {
        if curve.is_empty() {
            continue;
        }
        let mut previous = curve.last().unwrap().controls[2];
        write!(text, "M{}", p(previous)).unwrap();
        for s in curve {
            let v = s.controls;
            if s.corner {
                write!(text, "L{}", p(v[2])).unwrap();
            } else if s.quadratic {
                let q = crate::outline::Point {
                    x: previous.x + 1.5 * (v[0].x - previous.x),
                    y: previous.y + 1.5 * (v[0].y - previous.y),
                };
                write!(text, "Q{} {}", p(q), p(v[2])).unwrap();
            } else {
                write!(text, "C{} {} {}", p(v[0]), p(v[1]), p(v[2])).unwrap();
            }
            previous = v[2];
        }
        text.push('Z');
    }
    text
}
fn round_curve(curve: &mut Curve, height: usize, precision: u8) {
    if curve.is_empty() {
        return;
    }
    let scale = 10f64.powi(precision as i32);
    let round = |p: &mut crate::outline::Point| {
        p.x = (p.x * scale).round() / scale;
        p.y = height as f64 - ((height as f64 - p.y) * scale).round() / scale;
    };
    let mut original = curve.last().unwrap().controls[2];
    let mut start = original;
    round(&mut start);
    for s in curve {
        let end = s.controls[2];
        if s.quadratic {
            let mut q = crate::outline::Point {
                x: original.x + 1.5 * (s.controls[0].x - original.x),
                y: original.y + 1.5 * (s.controls[0].y - original.y),
            };
            round(&mut q);
            round(&mut s.controls[2]);
            let end = s.controls[2];
            s.controls[0] = crate::outline::Point {
                x: start.x + (q.x - start.x) * 2. / 3.,
                y: start.y + (q.y - start.y) * 2. / 3.,
            };
            s.controls[1] = crate::outline::Point {
                x: end.x + (q.x - end.x) * 2. / 3.,
                y: end.y + (q.y - end.y) * 2. / 3.,
            };
        } else {
            for p in &mut s.controls {
                round(p);
            }
        }
        original = end;
        start = s.controls[2];
    }
}
pub(crate) fn round_scene(layers: &mut [TraceLayer], height: usize, precision: u8) {
    for layer in layers {
        for (i, curve) in layer.curves.iter_mut().enumerate() {
            if layer.svg_shapes.get(i).is_some_and(Option::is_some) {
                continue;
            }
            round_curve(curve, height, precision);
        }
        if let Some(clip) = &mut layer.clip {
            for curve in clip {
                round_curve(curve, height, precision);
            }
        }
    }
}
pub(crate) fn layers(layers: &[TraceLayer], height: usize, precision: u8) -> String {
    let mut text = String::new();
    for (i, layer) in layers.iter().enumerate() {
        if layer.curves.is_empty() {
            continue;
        }
        if let Some(g) = &layer.gradient {
            let tag = if g.radial {
                "radialGradient"
            } else {
                "linearGradient"
            };
            write!(
                text,
                "<defs><{tag} id=\"g{i}\" gradientUnits=\"userSpaceOnUse\" "
            )
            .unwrap();
            if g.radial {
                write!(
                    text,
                    "cx=\"{}\" cy=\"{}\" r=\"{}\"",
                    g.from[0], g.from[1], g.to[0]
                )
                .unwrap();
            } else {
                write!(
                    text,
                    "x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"",
                    g.from[0], g.from[1], g.to[0], g.to[1]
                )
                .unwrap();
            }
            text.push('>');
            for (offset, c) in &g.stops {
                write!(
                    text,
                    "<stop offset=\"{offset:.3}\" stop-color=\"#{:02x}{:02x}{:02x}\"/>",
                    c[0], c[1], c[2]
                )
                .unwrap();
            }
            write!(text, "</{tag}></defs>").unwrap();
        }
        if let Some(clip) = &layer.clip {
            write!(text,"<defs><clipPath id=\"clip{i}\"><path fill-rule=\"evenodd\" clip-rule=\"evenodd\" d=\"{}\"/></clipPath></defs>",path(clip,height,precision)).unwrap();
        }
        let color = if layer.gradient.is_some() {
            format!("url(#g{i})")
        } else {
            format!(
                "#{:02x}{:02x}{:02x}",
                layer.color[0], layer.color[1], layer.color[2]
            )
        };
        let fill = if layer.stroke_only { "none" } else { &color };
        let mut attrs = format!(
            "fill=\"{fill}\" fill-opacity=\"{:.4}\" fill-rule=\"evenodd\"",
            layer.color[3] as f64 / 255.
        );
        if layer.clip.is_some() {
            write!(attrs, " clip-path=\"url(#clip{i})\"").unwrap();
        }
        if layer.stroke > 0. {
            write!(attrs," stroke=\"{color}\" stroke-opacity=\"{:.4}\" stroke-width=\"{}\" stroke-linejoin=\"round\"",layer.color[3] as f64/255.,layer.stroke).unwrap();
        }
        let curves: Vec<_> = layer
            .curves
            .iter()
            .enumerate()
            .filter(|(i, _)| !layer.svg_shapes.get(*i).is_some_and(Option::is_some))
            .map(|(_, curve)| curve.clone())
            .collect();
        if !curves.is_empty() {
            write!(
                text,
                "<path {attrs} d=\"{}\"/>",
                path(&curves, height, precision)
            )
            .unwrap();
        }
        for shape in layer.svg_shapes.iter().flatten() {
            text.push_str(&shape.replace("__PAINT__", &attrs));
        }
    }
    text
}
