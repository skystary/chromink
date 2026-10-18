//! One RGBA8 sampling path: positive area weights for shrinking, linear for enlargement.
//! Interpolate premultiplied color so hidden RGB cannot stain transparent edges.
use crate::{control::WorkControl, parallel::Parallelism};
use image::RgbaImage;
use rayon::prelude::*;

struct Span {
    first: usize,
    last: usize,
    left: f64,
    right: f64,
    scale: f64,
    area: bool,
}
impl Span {
    fn weight(&self, pixel: usize) -> f32 {
        if self.area {
            ((((pixel + 1) as f64).min(self.right) - self.left.max(pixel as f64)) * self.scale)
                as f32
        } else if self.first == self.last {
            1.
        } else if pixel == self.first {
            (1. - self.left) as f32
        } else {
            self.left as f32
        }
    }
}
fn axis(input: u32, output: u32) -> Vec<Span> {
    let ratio = input as f64 / output as f64;
    (0..output)
        .map(|i| {
            if output < input {
                let left = i as f64 * ratio;
                let right = ((i + 1) as f64 * ratio).min(input as f64);
                Span {
                    first: left.floor() as usize,
                    last: (right.ceil() as usize - 1).min(input as usize - 1),
                    left,
                    right,
                    scale: 1. / (right - left),
                    area: true,
                }
            } else {
                let position = ((i as f64 + 0.5) * ratio - 0.5).clamp(0., input as f64 - 1.);
                let first = position.floor() as usize;
                Span {
                    first,
                    last: (first + 1).min(input as usize - 1),
                    left: position - first as f64,
                    right: 0.,
                    scale: 1.,
                    area: false,
                }
            }
        })
        .collect()
}
pub fn resize(
    source: &RgbaImage,
    width: u32,
    height: u32,
    workers: &Parallelism,
    control: &WorkControl,
) -> Result<RgbaImage, String> {
    control.check()?;
    crate::control::validate_decode(width, height)?;
    if source.width() == 0 || source.height() == 0 {
        return Err("输入像素尺寸无效".into());
    }
    let _memory = control.reserve(
        width as usize * height as usize * 4
            + (width as usize + height as usize) * std::mem::size_of::<Span>(),
    )?;
    if source.dimensions() == (width, height) {
        return Ok(source.clone());
    }
    let xs = axis(source.width(), width);
    let ys = axis(source.height(), height);
    let stride = source.width() as usize * 4;
    let raw = source.as_raw();
    let mut output = vec![0; width as usize * height as usize * 4];
    workers.install(|| {
        output
            .par_chunks_mut(width as usize * 4)
            .enumerate()
            .try_for_each(|(row, dest)| -> Result<(), String> {
                control.check()?;
                let y = &ys[row];
                for (column, pixel) in dest.chunks_exact_mut(4).enumerate() {
                    if column & 4095 == 0 {
                        control.check()?;
                    }
                    let x = &xs[column];
                    let mut sum = [0f32; 4];
                    for sy in y.first..=y.last {
                        let wy = y.weight(sy);
                        for sx in x.first..=x.last {
                            let weight = wy * x.weight(sx);
                            let offset = sy * stride + sx * 4;
                            let alpha = raw[offset + 3] as f32 * weight;
                            sum[3] += alpha;
                            for channel in 0..3 {
                                sum[channel] += raw[offset + channel] as f32 * alpha;
                            }
                        }
                    }
                    let alpha = sum[3].round().clamp(0., 255.) as u8;
                    if alpha != 0 {
                        for channel in 0..3 {
                            pixel[channel] = (sum[channel] / sum[3]).round().clamp(0., 255.) as u8;
                        }
                        pixel[3] = alpha;
                    }
                }
                Ok(())
            })
    })?;
    control.check()?;
    RgbaImage::from_raw(width, height, output).ok_or_else(|| "缩放像素尺寸无效".into())
}
