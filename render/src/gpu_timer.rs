use crate::readback::Readback;

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
    readback: Readback,
    period_ns: f32,
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
        Some(Self {
            query_set,
            resolve_buffer,
            readback: Readback::new(device, "timestamp readback", BUFFER_SIZE),
            period_ns: queue.get_timestamp_period(),
            latest: None,
        })
    }

    /// Timestamp writes for `pass`, or `None` if this frame is not being timed.
    pub fn pass_writes(&self, pass: Pass) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
        if !self.readback.is_idle() {
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
    pub fn resolve(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if !self.readback.is_idle() {
            return;
        }
        encoder.resolve_query_set(&self.query_set, 0..QUERY_COUNT, &self.resolve_buffer, 0);
        self.readback.copy_from(encoder, &self.resolve_buffer);
    }

    /// Starts the async readback; call right after the frame is submitted.
    pub fn after_submit(&mut self) {
        self.readback.after_submit();
    }

    /// Collects a finished measurement, if any. Never blocks.
    pub fn poll(&mut self, device: &wgpu::Device) {
        let Some(ticks) = self.readback.try_read::<u64>(device) else {
            return;
        };
        let ms = |pass: Pass| {
            let i = pass as usize * 2;
            ticks[i + 1].saturating_sub(ticks[i]) as f32 * self.period_ns / 1.0e6
        };
        self.latest = Some(GpuTimings {
            render_ms: ms(Pass::Render),
            display_ms: ms(Pass::Display),
        });
    }

    pub fn latest(&self) -> Option<GpuTimings> {
        self.latest
    }
}
