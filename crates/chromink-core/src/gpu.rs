//! Exact integer palette assignment on native hardware compute backends.
//! The device and shader are cached; failures disable it and preserve CPU fallback.
use crate::{control::WorkControl, TRANSPARENT};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, OnceLock,
};
use wgpu::util::DeviceExt;

static GPU: OnceLock<Mutex<Option<Gpu>>> = OnceLock::new();
static INITIALIZING: AtomicBool = AtomicBool::new(false);
const CHUNK: usize = 1_048_576;
struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    name: String,
    chunk: usize,
    workgroup: u32,
    failed: Arc<AtomicBool>,
}
fn cached() -> &'static Mutex<Option<Gpu>> {
    GPU.get_or_init(|| Mutex::new(std::panic::catch_unwind(Gpu::new).ok().flatten()))
}
pub fn adapter_name() -> Option<String> {
    cached().lock().ok()?.as_ref().map(|gpu| gpu.name.clone())
}
pub fn try_remap(input: &[[u8; 4]], palette: &[[u8; 4]]) -> Option<Vec<u16>> {
    try_remap_controlled(input, palette, &WorkControl::unlimited())
}
pub fn try_remap_controlled(
    input: &[[u8; 4]],
    palette: &[[u8; 4]],
    control: &WorkControl,
) -> Option<Vec<u16>> {
    if std::env::var_os("CHROMINK_GPU").is_some_and(|value| value == "off")
        || input.len() < 1_048_576
        || palette.len() < 64
    {
        return None;
    }
    // Flat scans and illustrations benefit more from the CPU's color cache.
    let sample: std::collections::HashSet<_> = input
        .iter()
        .step_by((input.len() / 4096).max(1))
        .take(4096)
        .copied()
        .collect();
    if sample.len() <= 256 {
        return None;
    }
    // Never put driver startup on an automatic preview's critical path. Explicit
    // backend requests and map_exact remain synchronous for diagnostics/benchmarks.
    let explicit = std::env::var("CHROMINK_GPU_BACKEND")
        .is_ok_and(|v| matches!(v.as_str(), "dx12" | "vulkan" | "metal"));
    if !explicit && GPU.get().is_none() {
        if INITIALIZING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            if std::thread::Builder::new()
                .name("chromink-gpu-init".into())
                .spawn(|| {
                    let _ = cached();
                })
                .is_err()
            {
                INITIALIZING.store(false, Ordering::Release);
            }
        }
        return None;
    }
    map_controlled(input, palette, control)
}
/// Also exposed for reference validation and benchmarks without the size cutoff.
pub fn map_exact(input: &[[u8; 4]], palette: &[[u8; 4]]) -> Option<Vec<u16>> {
    map_controlled(input, palette, &WorkControl::unlimited())
}
fn map_controlled(
    input: &[[u8; 4]],
    palette: &[[u8; 4]],
    control: &WorkControl,
) -> Option<Vec<u16>> {
    if input.is_empty() {
        return Some(Vec::new());
    }
    if palette.is_empty() || palette.len() > 256 {
        return None;
    }
    let mut slot = cached().lock().ok()?;
    let gpu = slot.as_ref()?;
    let mapped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        gpu.map(input, palette, control)
    }))
    .ok()
    .flatten();
    if mapped.is_none() && !control.cancelled() {
        *slot = None;
    }
    mapped
}
impl Gpu {
    fn new() -> Option<Self> {
        if std::env::var_os("CHROMINK_GPU").is_some_and(|v| v == "off") {
            return None;
        }
        let backends = native_backends();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let mut adapters = instance.enumerate_adapters(backends);
        diagnostic(format_args!("available adapters: {}", adapters.len()));
        adapters.sort_by_key(|a| {
            let info = a.get_info();
            let kind = match info.device_type {
                wgpu::DeviceType::DiscreteGpu => 0,
                wgpu::DeviceType::IntegratedGpu => 1,
                _ => 2,
            };
            (
                kind,
                if info.backend == wgpu::Backend::Dx12 {
                    0
                } else {
                    1
                },
            )
        });
        for adapter in adapters {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                Self::from_adapter(&adapter)
            }))
            .ok()
            .flatten();
            if result.is_some() {
                return result;
            }
        }
        None
    }
    fn from_adapter(adapter: &wgpu::Adapter) -> Option<Self> {
        let info = adapter.get_info();
        diagnostic(format_args!(
            "trying {} {:?} {:?}",
            info.name, info.backend, info.device_type
        ));
        if !matches!(
            info.device_type,
            wgpu::DeviceType::DiscreteGpu | wgpu::DeviceType::IntegratedGpu
        ) || !adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
        {
            return None;
        }
        let limits = adapter.limits();
        let (chunk, workgroup) = geometry(&limits)?;
        let mut required = wgpu::Limits::downlevel_defaults().using_alignment(limits.clone());
        required.max_compute_invocations_per_workgroup = workgroup;
        required.max_compute_workgroup_size_x = workgroup;
        required.max_compute_workgroups_per_dimension = (chunk as u32).div_ceil(workgroup);
        required.max_storage_buffer_binding_size = (chunk * 4) as u32;
        required.max_buffer_size = (chunk * 4) as u64;
        required.max_storage_buffers_per_shader_stage = 3;
        if !required.check_limits(&limits) {
            diagnostic(format_args!(
                "required limits exceed {}: {required:?} available {limits:?}",
                info.name
            ));
            return None;
        }
        let (device, queue) = match pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("Chromink compute"),
                required_features: wgpu::Features::empty(),
                required_limits: required,
            },
            None,
        )) {
            Ok(result) => result,
            Err(e) => {
                diagnostic(format_args!("device creation: {e}"));
                return None;
            }
        };
        let failed = Arc::new(AtomicBool::new(false));
        let failure = failed.clone();
        device.on_uncaptured_error(Box::new(move |error| {
            diagnostic(format_args!("device error: {error}"));
            failure.store(true, Ordering::Relaxed);
        }));
        let loss = failed.clone();
        device.set_device_lost_callback(move |reason, message| {
            diagnostic(format_args!("device lost: {reason:?}: {message}"));
            loss.store(true, Ordering::Relaxed);
        });
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let source = include_str!("nearest.wgsl").replace(
            "@workgroup_size(256)",
            &format!("@workgroup_size({workgroup})"),
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Exact RGBA assignment"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Chromink palette mapping"),
            layout: None,
            module: &shader,
            entry_point: "main",
            compilation_options: Default::default(),
        });
        if let Some(error) = pollster::block_on(device.pop_error_scope()) {
            diagnostic(format_args!("pipeline error: {error}"));
            return None;
        }
        if failed.load(Ordering::Relaxed) {
            return None;
        }
        let gpu = Self {
            device,
            queue,
            pipeline,
            name: format!("{} ({:?})", info.name, info.backend),
            chunk,
            workgroup,
            failed,
        };
        // Driver/backend smoke check before routing real work; first-index ties and transparent sentinel.
        let sample = [[1, 1, 1, 255], [255, 0, 127, 0], [255; 4], [0, 0, 0, 255]];
        if gpu.map(
            &sample,
            &[[0, 0, 0, 255], [2, 2, 2, 255]],
            &WorkControl::unlimited(),
        )? != vec![0, TRANSPARENT, 1, 0]
        {
            return None;
        }
        Some(gpu)
    }
    fn map(
        &self,
        input: &[[u8; 4]],
        palette: &[[u8; 4]],
        control: &WorkControl,
    ) -> Option<Vec<u16>> {
        if self.failed.load(Ordering::Relaxed) {
            return None;
        }
        let colors: &[u8] = bytemuck::cast_slice(palette);
        let palette_buffer = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: None,
                contents: colors,
                usage: wgpu::BufferUsages::STORAGE,
            });
        let layout = self.pipeline.get_bind_group_layout(0);
        let mut result = Vec::with_capacity(input.len());
        for pixels in input.chunks(self.chunk) {
            if control.cancelled() {
                return None;
            }
            let bytes: &[u8] = bytemuck::cast_slice(pixels);
            let size = bytes.len() as u64;
            let _scratch = control
                .reserve(bytes.len().saturating_mul(3).saturating_add(colors.len()))
                .ok()?;
            let source = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytes,
                    usage: wgpu::BufferUsages::STORAGE,
                });
            let output = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let parameters: Vec<u8> = [pixels.len() as u32, palette.len() as u32, 0, 0]
                .into_iter()
                .flat_map(u32::to_le_bytes)
                .collect();
            let counts = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: &parameters,
                    usage: wgpu::BufferUsages::UNIFORM,
                });
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: source.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: palette_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: output.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: counts.as_entire_binding(),
                    },
                ],
            });
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: None,
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups((pixels.len() as u32).div_ceil(self.workgroup), 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
            self.queue.submit(Some(encoder.finish()));
            let slice = readback.slice(..);
            let (sender, receiver) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |mapped| {
                let _ = sender.send(mapped);
            });
            // Poll without blocking indefinitely on a lost or unresponsive device.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                if control.cancelled() || self.failed.load(Ordering::Relaxed) {
                    return None;
                }
                self.device.poll(wgpu::Maintain::Poll);
                match receiver.try_recv() {
                    Ok(mapped) => {
                        mapped.ok()?;
                        break;
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => return None,
                    Err(_) if std::time::Instant::now() >= deadline => return None,
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(1)),
                }
            }
            {
                let mapped = slice.get_mapped_range();
                for value in mapped.chunks_exact(4) {
                    let index = u32::from_le_bytes(value.try_into().ok()?);
                    if index != u32::MAX && index as usize >= palette.len() {
                        return None;
                    }
                    result.push(if index == u32::MAX {
                        TRANSPARENT
                    } else {
                        index as u16
                    });
                }
            }
            readback.unmap();
        }
        Some(result)
    }
}

fn native_backends() -> wgpu::Backends {
    let requested = std::env::var("CHROMINK_GPU_BACKEND").unwrap_or_else(|_| "auto".into());
    let selected = match requested.as_str() {
        "dx12" => Some(wgpu::Backends::DX12),
        "vulkan" => Some(wgpu::Backends::VULKAN),
        "metal" => Some(wgpu::Backends::METAL),
        "auto" => None,
        _ => Some(wgpu::Backends::empty()),
    };
    if let Some(selected) = selected {
        return selected;
    }
    #[cfg(target_os = "windows")]
    {
        wgpu::Backends::DX12 | wgpu::Backends::VULKAN
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        wgpu::Backends::METAL
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "ios")))]
    {
        wgpu::Backends::VULKAN
    }
}
fn geometry(limits: &wgpu::Limits) -> Option<(usize, u32)> {
    let supported = limits
        .max_compute_invocations_per_workgroup
        .min(limits.max_compute_workgroup_size_x)
        .min(256);
    if supported < 32
        || limits.max_storage_buffers_per_shader_stage < 3
        || limits.max_compute_workgroups_per_dimension == 0
    {
        return None;
    }
    let workgroup = 1u32 << (31 - supported.leading_zeros());
    let chunk = CHUNK
        .min(limits.max_storage_buffer_binding_size as usize / 4)
        .min((limits.max_buffer_size / 4).min(usize::MAX as u64) as usize)
        .min(
            (u64::from(limits.max_compute_workgroups_per_dimension) * u64::from(workgroup))
                .min(usize::MAX as u64) as usize,
        );
    // 256-entry palette requires 1024 bytes even when dispatch buffers are smaller.
    if chunk < 256 {
        return None;
    }
    Some((chunk, workgroup))
}

fn diagnostic(message: std::fmt::Arguments<'_>) {
    if std::env::var_os("CHROMINK_GPU_LOG").is_some_and(|v| v == "1") {
        eprintln!("Chromink GPU: {message}");
    }
}
