//! Exact integer RGBA matching. Dither feedback keeps its existing floating-point path.
//! Channels are expanded once per palette, not once per pixel/candidate.
pub(crate) struct Matcher {
    channels: [[i32; 256]; 4],
    len: usize,
    kernel: Kernel,
}
#[derive(Clone, Copy)]
enum Kernel {
    Scalar,
    #[cfg(target_arch = "x86_64")]
    Avx2Asm,
    #[cfg(target_arch = "aarch64")]
    Neon,
}
impl Matcher {
    pub(crate) fn new(palette: &[[u8; 4]]) -> Self {
        assert!(!palette.is_empty() && palette.len() <= 256);
        let mut channels = [[0; 256]; 4];
        for (i, color) in palette.iter().enumerate() {
            for c in 0..4 {
                channels[c][i] = i32::from(color[c]);
            }
        }
        let kernel = Kernel::Scalar;
        #[cfg(target_arch = "x86_64")]
        let kernel = if palette.len() >= 16 && std::is_x86_feature_detected!("avx2") {
            Kernel::Avx2Asm
        } else {
            kernel
        };
        #[cfg(target_arch = "aarch64")]
        let kernel = if palette.len() >= 16 {
            Kernel::Neon
        } else {
            kernel
        };
        Self {
            channels,
            len: palette.len(),
            kernel,
        }
    }
    #[inline]
    pub(crate) fn nearest(&self, pixel: [u8; 4]) -> u16 {
        match self.kernel {
            Kernel::Scalar => self.scalar(pixel),
            #[cfg(target_arch = "x86_64")]
            // Constructor verified AVX2 + OS support. Arrays always contain 256 initialized lanes.
            Kernel::Avx2Asm => unsafe { self.avx2(pixel, true) },
            #[cfg(target_arch = "aarch64")]
            // NEON is baseline on AArch64. No optional CPU extensions used.
            Kernel::Neon => unsafe { self.neon(pixel) },
        }
    }
    fn scalar(&self, pixel: [u8; 4]) -> u16 {
        let mut best = 0;
        let mut minimum = i32::MAX;
        for i in 0..self.len {
            let mut distance = 0;
            for c in 0..4 {
                let d = i32::from(pixel[c]) - self.channels[c][i];
                distance += d * d;
            }
            if distance < minimum {
                minimum = distance;
                best = i;
            }
        }
        best as u16
    }
    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "avx2")]
    unsafe fn avx2(&self, pixel: [u8; 4], assembly: bool) -> u16 {
        use std::arch::x86_64::*;
        let p: [__m256i; 4] = pixel.map(|c| _mm256_set1_epi32(i32::from(c)));
        let mut minimum = i32::MAX;
        let mut best = 0;
        for start in (0..self.len).step_by(8) {
            let mut scores = [0i32; 8];
            if assembly {
                // Each pointer has at least 8 valid i32 lanes (last block is padded to 256).
                // All scratch registers and memory writes are declared; no stack or flags used.
                unsafe {
                    std::arch::asm!(
                        "vpsubd {acc}, {pr}, ymmword ptr [{r}]",
                        "vpmulld {acc}, {acc}, {acc}",
                        "vpsubd {tmp}, {pg}, ymmword ptr [{g}]",
                        "vpmulld {tmp}, {tmp}, {tmp}",
                        "vpaddd {acc}, {acc}, {tmp}",
                        "vpsubd {tmp}, {pb}, ymmword ptr [{b}]",
                        "vpmulld {tmp}, {tmp}, {tmp}",
                        "vpaddd {acc}, {acc}, {tmp}",
                        "vpsubd {tmp}, {pa}, ymmword ptr [{a}]",
                        "vpmulld {tmp}, {tmp}, {tmp}",
                        "vpaddd {acc}, {acc}, {tmp}",
                        "vmovdqu ymmword ptr [{output}], {acc}",
                        pr=in(ymm_reg) p[0], pg=in(ymm_reg) p[1], pb=in(ymm_reg) p[2], pa=in(ymm_reg) p[3],
                        r=in(reg) self.channels[0].as_ptr().add(start), g=in(reg) self.channels[1].as_ptr().add(start),
                        b=in(reg) self.channels[2].as_ptr().add(start), a=in(reg) self.channels[3].as_ptr().add(start),
                        output=in(reg) scores.as_mut_ptr(), acc=out(ymm_reg) _, tmp=out(ymm_reg) _,
                        options(nostack, preserves_flags),
                    );
                }
            } else {
                let mut sum = _mm256_setzero_si256();
                for c in 0..4 {
                    let channel =
                        unsafe { _mm256_loadu_si256(self.channels[c].as_ptr().add(start).cast()) };
                    let d = _mm256_sub_epi32(p[c], channel);
                    sum = _mm256_add_epi32(sum, _mm256_mullo_epi32(d, d));
                }
                unsafe {
                    _mm256_storeu_si256(scores.as_mut_ptr().cast(), sum);
                }
            }
            for (lane, &distance) in scores.iter().enumerate().take((self.len - start).min(8)) {
                if distance < minimum {
                    minimum = distance;
                    best = start + lane;
                }
            }
        }
        best as u16
    }
    #[cfg(target_arch = "aarch64")]
    #[target_feature(enable = "neon")]
    unsafe fn neon(&self, pixel: [u8; 4]) -> u16 {
        use std::arch::aarch64::*;
        let mut minimum = i32::MAX;
        let mut best = 0;
        for start in (0..self.len).step_by(4) {
            let mut sum = vdupq_n_s32(0);
            for c in 0..4 {
                let d = vsubq_s32(vdupq_n_s32(i32::from(pixel[c])), unsafe {
                    vld1q_s32(self.channels[c].as_ptr().add(start))
                });
                sum = vaddq_s32(sum, vmulq_s32(d, d));
            }
            let mut scores = [0; 4];
            unsafe {
                vst1q_s32(scores.as_mut_ptr(), sum);
            }
            for (lane, &distance) in scores.iter().enumerate().take((self.len - start).min(4)) {
                if distance < minimum {
                    minimum = distance;
                    best = start + lane;
                }
            }
        }
        best as u16
    }
    #[cfg(all(test, target_arch = "x86_64"))]
    pub(crate) fn intrinsic(&self, pixel: [u8; 4]) -> u16 {
        if std::is_x86_feature_detected!("avx2") {
            unsafe { self.avx2(pixel, false) }
        } else {
            self.scalar(pixel)
        }
    }
}
