use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// GPU time spent in each pass of the most recently measured frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct GpuTimings {
    pub render_ms: f32,
    pub display_ms: f32,
}

/// Measured passes, in query-slot order.
#[derive(Clone, Copy)]
pub(crate) enum Pass {
    Render = 0,
    Display = 1,
}
const PASS_COUNT: u32 = 2;
const QUERY_COUNT: u32 = PASS_COUNT * 2;
const BUFFER_SIZE: u64 = QUERY_COUNT as u64 * size_of::<u64>() as u64;

/// Timestamp-query based pass timer.
///
/// Readback is asynchronous and never stalls: while a measurement is in
/// flight, new frames are simply not timed.
pub(crate) struct GpuTimer {
    query_set: wgpu::QuerySet,
    resolve_buffer: wgpu::Buffer,
    readback_buffer: wgpu::Buffer,
    period_ns: f32,
    in_flight: bool,
    ready: Arc<AtomicBool>,
    latest: Option<GpuTimings>,
}

impl GpuTimer {
    /// Returns `None` when the device was created without `TIMESTAMP_QUERY`.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("pass timestamps"),
            ty: wgpu::QueryType::Timestamp,
            count: QUERY_COUNT,
        });
        let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("timestamp resolve"),
            size: BUFFER_SIZE,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("timestamp readback"),
            size: BUFFER_SIZE,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Some(Self {
            query_set,
            resolve_buffer,
            readback_buffer,
            period_ns: queue.get_timestamp_period(),
            in_flight: false,
            ready: Arc::new(AtomicBool::new(false)),
            latest: None,
        })
    }

    /// Timestamp writes for `pass`, or `None` if this frame is not being timed.
    pub fn pass_writes(&self, pass: Pass) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        if self.in_flight {
            return None;
        }
        let base = pass as u32 * 2;
        Some(wgpu::RenderPassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: Some(base),
            end_of_pass_write_index: Some(base + 1),
        })
    }

    /// Records the query resolve; call after all timed passes are encoded.
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        if self.in_flight {
            return;
        }
        encoder.resolve_query_set(&self.query_set, 0..QUERY_COUNT, &self.resolve_buffer, 0);
        encoder.copy_buffer_to_buffer(
            &self.resolve_buffer,
            0,
            &self.readback_buffer,
            0,
            BUFFER_SIZE,
        );
    }

    /// Starts the async readback; call right after the frame is submitted.
    pub fn after_submit(&mut self) {
        if self.in_flight {
            return;
        }
        self.in_flight = true;
        let ready = Arc::clone(&self.ready);
        self.readback_buffer
            .map_async(wgpu::MapMode::Read, .., move |result| {
                if result.is_ok() {
                    ready.store(true, Ordering::Release);
                }
            });
    }

    /// Collects a finished measurement, if any. Never blocks.
    pub fn poll(&mut self, device: &wgpu::Device) {
        if !self.in_flight {
            return;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        if !self.ready.swap(false, Ordering::Acquire) {
            return;
        }
        if let Ok(data) = self.readback_buffer.get_mapped_range(..) {
            let ticks: &[u64] = bytemuck::cast_slice(&data);
            let ms = |pass: Pass| {
                let i = pass as usize * 2;
                ticks[i + 1].saturating_sub(ticks[i]) as f32 * self.period_ns / 1.0e6
            };
            self.latest = Some(GpuTimings {
                render_ms: ms(Pass::Render),
                display_ms: ms(Pass::Display),
            });
        }
        self.readback_buffer.unmap();
        self.in_flight = false;
    }

    pub fn latest(&self) -> Option<GpuTimings> {
        self.latest
    }
}
