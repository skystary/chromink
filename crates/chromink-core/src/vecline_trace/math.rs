//! Shared perceptual conversion and small dense least-squares solver.
pub(super) fn srgb(p: [f64; 3]) -> [u8; 3] {
    let [l, a, b] = p;
    let ll = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s = (p[0] - 0.0894841775 * p[1] - 1.2914855480 * p[2]).powi(3);
    [
        4.0767416621 * ll - 3.3077115913 * m + 0.2309699292 * s,
        -1.2684380046 * ll + 2.6097574011 * m - 0.3413193965 * s,
        -0.0041960863 * ll - 0.7034186147 * m + 1.7076147010 * s,
    ]
    .map(|v| {
        let v = if v <= 0.0031308 {
            12.92 * v
        } else {
            1.055 * v.powf(1. / 2.4) - 0.055
        };
        (v * 255.).round().clamp(0., 255.) as u8
    })
}
pub(super) fn solve(mut a: [[f64; 3]; 3], mut b: [f64; 3]) -> Option<[f64; 3]> {
    for col in 0..3 {
        let pivot = (col..3)
            .max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))
            .unwrap();
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        let d = a[col][col];
        for j in col..3 {
            a[col][j] /= d;
        }
        b[col] /= d;
        for i in 0..3 {
            if i == col {
                continue;
            }
            let d = a[i][col];
            for j in col..3 {
                a[i][j] -= d * a[col][j];
            }
            b[i] -= d * b[col];
        }
    }
    Some(b)
}
