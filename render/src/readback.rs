use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    /// A copy into the buffer has been encoded but not yet submitted.
    Recorded,
    /// Submitted and waiting for `map_async` to complete.
    Mapping,
}

/// Non-blocking GPU → CPU readback of a small buffer.
///
/// Usage per frame: `copy_from` (only while idle) → submit → `after_submit`
/// → `try_read` on later frames until it yields data.
pub(crate) struct Readback {
    buffer: wgpu::Buffer,
    size: u64,
    state: State,
    ready: Arc<AtomicBool>,
}

impl Readback {
    pub fn new(device: &wgpu::Device, label: &str, size: u64) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            size,
            state: State::Idle,
            ready: Arc::new(AtomicBool::new(false)),
        }
    }

    /// `true` when a new copy may be recorded.
    pub fn is_idle(&self) -> bool {
        self.state == State::Idle
    }

    pub fn copy_from(&mut self, encoder: &mut wgpu::CommandEncoder, source: &wgpu::Buffer) {
        if self.state != State::Idle {
            return;
        }
        encoder.copy_buffer_to_buffer(source, 0, &self.buffer, 0, self.size);
        self.state = State::Recorded;
    }

    /// Call after the encoder holding the copy has been submitted.
    pub fn after_submit(&mut self) {
        if self.state != State::Recorded {
            return;
        }
        self.state = State::Mapping;
        let ready = Arc::clone(&self.ready);
        self.buffer
            .map_async(wgpu::MapMode::Read, .., move |result| {
                if result.is_ok() {
                    ready.store(true, Ordering::Release);
                }
            });
    }

    /// Returns the buffer contents once the readback has completed.
    ///
    /// Must not be called between `copy_from` and the submit of that
    /// encoder: a recorded copy is assumed to be submitted and gets mapped.
    pub fn try_read<T: bytemuck::Pod>(&mut self, device: &wgpu::Device) -> Option<Vec<T>> {
        self.after_submit();
        if self.state != State::Mapping {
            return None;
        }
        let _ = device.poll(wgpu::PollType::Poll);
        if !self.ready.swap(false, Ordering::Acquire) {
            return None;
        }
        let values = self
            .buffer
            .get_mapped_range(..)
            .ok()
            .map(|data| bytemuck::cast_slice::<u8, T>(&data).to_vec());
        self.buffer.unmap();
        self.state = State::Idle;
        values
    }
}
