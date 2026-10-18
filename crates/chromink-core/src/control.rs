use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
pub const BUDGET: &str = "图像内存或轮廓复杂度超过当前预算，请减少颜色、缩小图片或增加斑点过滤";
pub const CANCELLED: &str = "描摹已取消";
#[derive(Clone)]
pub struct WorkControl {
    cancel: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    used: Arc<AtomicUsize>,
    limit: usize,
}
impl WorkControl {
    pub fn new(cancel: Arc<AtomicBool>, limit: usize) -> Self {
        Self {
            cancel,
            failed: Arc::new(AtomicBool::new(false)),
            used: Arc::new(AtomicUsize::new(0)),
            limit,
        }
    }
    pub fn unlimited() -> Self {
        Self::new(Arc::new(AtomicBool::new(false)), usize::MAX)
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed) || self.failed.load(Ordering::Relaxed)
    }
    pub fn check(&self) -> Result<(), String> {
        if self.failed.load(Ordering::Relaxed) {
            Err(BUDGET.into())
        } else if self.cancelled() {
            Err(CANCELLED.into())
        } else {
            Ok(())
        }
    }
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn claim(&self, bytes: usize) -> Result<(), String> {
        self.check()?;
        self.used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(bytes).filter(|total| *total <= self.limit)
            })
            .map(|_| ())
            .map_err(|_| {
                self.failed.store(true, Ordering::Relaxed);
                BUDGET.into()
            })
    }
    pub fn remaining(&self) -> usize {
        self.limit.saturating_sub(self.used.load(Ordering::Relaxed))
    }
    pub fn reserve(&self, bytes: usize) -> Result<Reservation, String> {
        self.claim(bytes)?;
        Ok(Reservation {
            control: self.clone(),
            bytes,
        })
    }
}
pub struct Reservation {
    control: WorkControl,
    bytes: usize,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.control.used.fetch_sub(self.bytes, Ordering::Relaxed);
    }
}
#[cfg(windows)]
pub fn memory_budget() -> usize {
    #[repr(C)]
    struct MemoryStatus {
        length: u32,
        load: u32,
        total: u64,
        available: u64,
        page_total: u64,
        page_available: u64,
        virtual_total: u64,
        virtual_available: u64,
        extended: u64,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
    }
    let mut status = MemoryStatus {
        length: std::mem::size_of::<MemoryStatus>() as u32,
        load: 0,
        total: 0,
        available: 0,
        page_total: 0,
        page_available: 0,
        virtual_total: 0,
        virtual_available: 0,
        extended: 0,
    };
    // Windows fills this fixed repr(C) POD structure synchronously; no pointers escape.
    if unsafe { GlobalMemoryStatusEx(&mut status) } != 0 {
        (status.available as usize / 3).clamp(16 << 20, 2 << 30)
    } else {
        512 << 20
    }
}
#[cfg(not(windows))]
pub fn memory_budget() -> usize {
    let available = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("MemAvailable:"))
                .and_then(|line| line.split_whitespace().nth(1)?.parse::<usize>().ok())
        });
    available
        .map(|kb| kb.saturating_mul(1024) / 3)
        .unwrap_or(512 << 20)
        .clamp(16 << 20, 2 << 30)
}
pub fn validate_decode(width: u32, height: u32) -> Result<(), String> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0
        || pixels > crate::MAX_PIXELS
        || pixels.saturating_mul(12) > memory_budget() as u64
    {
        return Err("输入图像超过当前可用内存预算，请缩小图片或释放内存".into());
    }
    Ok(())
}
