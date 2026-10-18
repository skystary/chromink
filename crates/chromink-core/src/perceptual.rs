//! Exact Oklab/alpha mapping, parallel and cached without a 16-million-cell table.
use crate::{control::WorkControl, vecline, TRANSPARENT};
use rayon::prelude::*;
pub(crate) struct Matcher {
    labs: Vec<[f64; 3]>,
    alpha: Vec<f64>,
}
impl Matcher {
    pub(crate) fn new(palette: &[[u8; 4]]) -> Self {
        Self {
            labs: palette
                .iter()
                .map(|p| vecline::lab([p[0], p[1], p[2]]))
                .collect(),
            alpha: palette.iter().map(|p| p[3] as f64 / 255.).collect(),
        }
    }
    pub(crate) fn nearest(&self, p: [f64; 4]) -> usize {
        let point = vecline::lab([p[0].round() as u8, p[1].round() as u8, p[2].round() as u8]);
        let mut best = 0;
        let mut score = f64::INFINITY;
        for i in 0..self.labs.len() {
            let d = vecline::distance(point, self.labs[i]) + (p[3] / 255. - self.alpha[i]).powi(2);
            if d < score {
                score = d;
                best = i;
            }
        }
        best
    }
    pub(crate) fn map(&self, input: &[[u8; 4]], control: &WorkControl) -> Result<Vec<u16>, String> {
        let mut result = vec![TRANSPARENT; input.len()];
        result
            .par_chunks_mut(65536)
            .zip(input.par_chunks(65536))
            .try_for_each(|(out, part)| {
                let mut keys = [0; 4096];
                let mut valid = [false; 4096];
                let mut cache = [0u16; 4096];
                for (i, (&p, target)) in part.iter().zip(out).enumerate() {
                    if i & 8191 == 0 {
                        control.check()?;
                    }
                    if p[3] == 0 {
                        continue;
                    }
                    let key = u32::from_le_bytes(p);
                    let slot = (key.wrapping_mul(0x9e3779b9) >> 20) as usize;
                    if !valid[slot] || keys[slot] != key {
                        valid[slot] = true;
                        keys[slot] = key;
                        cache[slot] = self.nearest(p.map(f64::from)) as u16;
                    }
                    *target = cache[slot];
                }
                Ok::<(), String>(())
            })?;
        Ok(result)
    }
}
/// Keep the source's mean coverage of each fitted ink; invisible RGB never votes.
/// Exactly representable RGBA images take a separate no-quantization fast path.
pub(crate) fn fit_alpha(
    palette: &mut [[u8; 4]],
    input: &[[u8; 4]],
    control: &WorkControl,
) -> Result<(), String> {
    if input.iter().all(|p| p[3] == 0 || p[3] == 255) {
        return Ok(());
    }
    let matcher = Matcher::new(palette);
    let ids = matcher.map(input, control)?;
    let mut sums = vec![(0u64, 0u64); palette.len()];
    for (i, (p, id)) in input.iter().zip(ids).enumerate() {
        if i & 8191 == 0 {
            control.check()?;
        }
        if id == TRANSPARENT {
            continue;
        }
        sums[id as usize].0 += p[3] as u64;
        sums[id as usize].1 += 1;
    }
    for (p, (sum, n)) in palette.iter_mut().zip(sums) {
        if n > 0 {
            p[3] = ((sum + n / 2) / n).clamp(1, 255) as u8;
        }
    }
    Ok(())
}
