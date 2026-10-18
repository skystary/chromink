//! Vecline's alpha Lloyd relaxation and spatial fringe collapse.
use crate::control::WorkControl;
fn levels(image: &image::RgbaImage, max_levels: u8) -> Vec<usize> {
    let mut hist = [0u64; 256];
    for p in image.pixels() {
        hist[p[3] as usize] += 1;
    }
    let present: Vec<_> = (0..256).filter(|&i| hist[i] > 0).collect();
    let k = max_levels.max(1) as usize;
    let levels = if present.len() <= k {
        present.clone()
    } else {
        let mut centers: Vec<_> = (0..k)
            .map(|i| {
                present[((i * (present.len() - 1)) as f64 / (k - 1).max(1) as f64).round() as usize]
            })
            .collect();
        for _ in 0..24 {
            let mut sums = vec![0u64; k];
            let mut counts = vec![0u64; k];
            for &v in &present {
                let best = (0..k)
                    .min_by_key(|&i| (v as isize - centers[i] as isize).abs())
                    .unwrap();
                sums[best] += v as u64 * hist[v];
                counts[best] += hist[v];
            }
            let mut next = centers.clone();
            for i in 0..k {
                if counts[i] > 0 {
                    next[i] = (sums[i] as f64 / counts[i] as f64).round() as usize;
                }
            }
            if next == centers {
                break;
            }
            centers = next;
        }
        if hist[0] > 0 {
            centers.push(0);
        }
        if hist[255] > 0 {
            centers.push(255);
        }
        centers.sort_unstable();
        centers.dedup();
        centers
    };

    levels
}
fn nearest(levels: &[usize], v: u8) -> usize {
    (0..levels.len())
        .min_by_key(|&j| (v as isize - levels[j] as isize).abs())
        .unwrap()
}
pub(super) fn collapse(
    image: &image::RgbaImage,
    max_levels: u8,
    c: &WorkControl,
) -> Result<image::RgbaImage, String> {
    let levels = levels(image, max_levels);
    if levels.len() <= 2 {
        return Ok(image.clone());
    }
    let (w, h) = (image.width() as usize, image.height() as usize);
    let _scratch = c.reserve(w * h * 8)?;
    let bands: Vec<_> = image.pixels().map(|p| nearest(&levels, p[3])).collect();
    let mut total = vec![0usize; levels.len()];
    let mut interior = total.clone();
    for y in 0..h {
        c.check()?;
        for x in 0..w {
            let i = y * w + x;
            let band = bands[i];
            total[band] += 1;
            if (x == 0 || bands[i - 1] == band)
                && (x + 1 == w || bands[i + 1] == band)
                && (y == 0 || bands[i - w] == band)
                && (y + 1 == h || bands[i + w] == band)
            {
                interior[band] += 1;
            }
        }
    }
    let keep: Vec<_> = levels
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            v == 0 || v == 255 || total[i] > 0 && interior[i] as f64 / total[i] as f64 >= 0.2
        })
        .collect();
    let kept: Vec<_> = levels
        .iter()
        .zip(&keep)
        .filter_map(|(&v, &k)| k.then_some(v))
        .collect();
    if kept.is_empty() || keep.iter().all(|&k| k) {
        return Ok(image.clone());
    }
    let remap: Vec<_> = levels
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            if keep[i] {
                v
            } else {
                kept[nearest(&kept, v as u8)]
            }
        })
        .collect();
    let mut out = image.clone();
    for (p, &band) in out.pixels_mut().zip(&bands) {
        p[3] = remap[band] as u8;
    }
    Ok(out)
}
pub(super) fn expand(
    palette: &[[u8; 4]],
    ids: &[u16],
    image: &image::RgbaImage,
    max_levels: u8,
    c: &WorkControl,
) -> Result<(Vec<[u8; 4]>, Vec<u16>), String> {
    let all = levels(image, max_levels);
    let levels: Vec<_> = all.iter().copied().filter(|&v| v > 0).collect();
    let mut expanded = Vec::new();
    for p in palette {
        for &alpha in &levels {
            expanded.push([p[0], p[1], p[2], alpha as u8]);
        }
    }
    let mapped = ids
        .iter()
        .zip(image.pixels())
        .enumerate()
        .map(|(i, (&id, p))| {
            if i & 8191 == 0 {
                c.check()?;
            }
            let alpha = all[nearest(&all, p[3])];
            Ok(if id == crate::TRANSPARENT || alpha == 0 {
                crate::TRANSPARENT
            } else {
                (id as usize * levels.len() + levels.iter().position(|&v| v == alpha).unwrap())
                    as u16
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok((expanded, mapped))
}
