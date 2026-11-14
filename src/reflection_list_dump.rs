//! Opt-in snapshot of one frame's glossy-pixel list (L) for a CPU replay of the reflection
//! pass's real rays (docs/reflection-geometry-dump.md). It shares the geometry dump's switch,
//! directory, frame and file stem, so the two snapshots of a frame pair up. Measurement only:
//! one copy after the trace, mapped on submission; no wait, shader, binding or key change.
use super::reflection_geometry_dump::{self as geometry_dump, DISK_FLOOR};
use super::*;
use std::sync::{Mutex, Weak};

/// Whether this process dumps. L and the frame targets then gain COPY_SRC (usage only).
pub(super) fn enabled() -> bool {
    geometry_dump::config().is_some()
}

/// The 48-byte entry, as `rl_trace_pixels` reads it (shaders/reflections_trace.wgsl).
const ENTRY_LAYOUT: &str = "a: origin.xyz f32 bits, w pack2x16float(cone width, spread); b: x oct normal, y oct view, z oct side, w low16 pack2x16float alpha, bits 16-20 strata count; c: x pixel (x | y << 16), y first slot, zw reserved";

/// On the dump frame, once per renderer: copies L and the receiver epsilon after the trace
/// and writes `<stem>.glossy.bin` (L's 16-byte header and its listed entries), `<stem>.glossy-
/// epsilon.bin` (each entry's receiver epsilon, f32 bits, the trace's tmin) and
/// `<stem>.glossy.json` when the map lands.
pub(super) fn maybe_dump(
    renderer: &Renderer,
    encoder: &mut wgpu::CommandEncoder,
    pass: &super::reflection_pass::PassTargets,
    epsilon: &wgpu::Texture,
) {
    let Some(config) = geometry_dump::config() else {
        return;
    };
    if renderer.frames != config.frame {
        return;
    }
    static ATTEMPTED: Mutex<Vec<Weak<()>>> = Mutex::new(Vec::new());
    let id = Arc::downgrade(&renderer.identity);
    {
        let mut attempted = ATTEMPTED.lock().unwrap();
        if attempted.iter().any(|a| a.ptr_eq(&id)) {
            return;
        }
        attempted.push(id);
    }
    let state = renderer.view.primary.as_ref().unwrap();
    let list_bytes = pass.list.value.size();
    let [width, height] = pass.size;
    let row = u64::from(width * 4).next_multiple_of(u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT));
    let epsilon_at = list_bytes.next_multiple_of(256);
    let bytes = epsilon_at + row * u64::from(height);
    match geometry_dump::free_space(&config.directory) {
        Ok(free) if free >= DISK_FLOOR.saturating_add(bytes) => {}
        free => {
            eprintln!("reflection_list_dump refused: needs {bytes} plus {DISK_FLOOR} bytes free; available={free:?}");
            return;
        }
    }
    // The trace's direction seed, primary_frame.sided.y, as written this frame.
    let rotation = state.frame.as_ref().map_or(0, |(_, words)| {
        u32::from_ne_bytes(words[84..88].try_into().unwrap())
    });
    let generation = state.reflections.as_ref().map_or(0, |r| r.generation);
    let stem = format!(
        "renderer-{:x}-view-{}-frame-{}-generation-{}",
        Arc::as_ptr(&renderer.identity) as usize,
        renderer.view_key.0,
        renderer.frames,
        generation,
    );
    let header = Header {
        path: if renderer.hardware_shadows.is_some() {
            "rt"
        } else {
            "nonrt"
        },
        frame: renderer.frames,
        view: renderer.view_key.0,
        generation,
        rotation,
        room: pass.room,
        size: pass.size,
    };
    let staging = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("reflection glossy-list dump (measurement only)"),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_buffer_to_buffer(&pass.list.value, 0, &staging, 0, list_bytes);
    encoder.copy_texture_to_buffer(
        epsilon.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: epsilon_at,
                bytes_per_row: Some(row as u32),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let mapped = staging.clone();
    let directory = config.directory.clone();
    encoder.map_buffer_on_submit(&staging, wgpu::MapMode::Read, .., move |result| {
        if result.is_err() {
            eprintln!("reflection_list_dump: map failed");
            return;
        }
        let Ok(data) = mapped.get_mapped_range(..) else {
            return;
        };
        let (list, epsilon) = data.split_at(epsilon_at as usize);
        let snapshot = Snapshot::new(list, header.room as usize, |[x, y]| {
            let at = y as usize * row as usize + x as usize * 4;
            u32::from_ne_bytes(epsilon[at..at + 4].try_into().unwrap())
        });
        match snapshot.write(&directory, &stem, &header) {
            Ok(written) => eprintln!(
                "reflection_list_dump path={} frame={} entries={} bytes={written} file={stem}.glossy.json",
                header.path,
                header.frame,
                snapshot.entries.len() / ENTRY_BYTES
            ),
            Err(error) => eprintln!("reflection_list_dump: {error}"),
        }
    });
}

const ENTRY_BYTES: usize = 48;
const HEADER_BYTES: usize = 16;

/// What the JSON names beside the entries.
struct Header {
    path: &'static str,
    frame: u64,
    view: u64,
    generation: u64,
    rotation: u32,
    room: u64,
    size: [u32; 2],
}

/// L's header and listed entries, and each entry's receiver epsilon bits.
struct Snapshot<'a> {
    header: &'a [u8],
    /// The length the shaders appended (may exceed the capacity; those receivers stayed inline).
    appended: u32,
    capacity: usize,
    entries: &'a [u8],
    epsilon: Vec<u32>,
}

impl<'a> Snapshot<'a> {
    /// `list`: L's bytes (at least its header and `capacity` entries); `capacity`: the entries
    /// the shaders may write (the room, `rl_pass_frame.z`); `epsilon`: the receiver epsilon bits
    /// at a pixel.
    fn new(list: &'a [u8], capacity: usize, epsilon: impl Fn([u32; 2]) -> u32) -> Self {
        let appended = u32::from_ne_bytes(list[..4].try_into().unwrap());
        let listed = (appended as usize).min(capacity);
        let entries = &list[HEADER_BYTES..HEADER_BYTES + listed * ENTRY_BYTES];
        let epsilon = entries
            .chunks_exact(ENTRY_BYTES)
            .map(|e| {
                let pixel = u32::from_ne_bytes(e[32..36].try_into().unwrap());
                epsilon([pixel & 0xffff, pixel >> 16])
            })
            .collect();
        Self {
            header: &list[..HEADER_BYTES],
            appended,
            capacity,
            entries,
            epsilon,
        }
    }

    /// Writes the payloads, then the JSON last (an interrupted write is not a snapshot); the
    /// bytes written.
    fn write(&self, directory: &std::path::Path, stem: &str, h: &Header) -> std::io::Result<u64> {
        std::fs::create_dir_all(directory)?;
        let list: Vec<u8> = [self.header, self.entries].concat();
        let epsilon: Vec<u8> = self.epsilon.iter().flat_map(|w| w.to_le_bytes()).collect();
        std::fs::write(directory.join(format!("{stem}.glossy.bin")), &list)?;
        std::fs::write(
            directory.join(format!("{stem}.glossy-epsilon.bin")),
            &epsilon,
        )?;
        let json = format!(
            "{{\"version\":1,\"path\":\"{}\",\"frame\":{},\"view\":{},\"generation\":{},\"primary_frame_sided_y\":{},\"room\":{},\"size\":[{},{}],\"appended\":{},\"capacity\":{},\"entries\":{},\"entry_bytes\":{ENTRY_BYTES},\"header_bytes\":{HEADER_BYTES},\"entry_layout\":\"{ENTRY_LAYOUT}\",\"epsilon\":\"f32 bits per entry, at its pixel\"}}\n",
            h.path,
            h.frame,
            h.view,
            h.generation,
            h.rotation,
            h.room,
            h.size[0],
            h.size[1],
            self.appended,
            self.capacity,
            self.epsilon.len(),
        );
        std::fs::write(directory.join(format!("{stem}.glossy.json")), &json)?;
        Ok((list.len() + epsilon.len() + json.len()) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_keeps_listed_entries_and_their_pixels_epsilon() {
        // Three appended, room for two: the third stayed inline and is not listed. The copy
        // is padded past the room, as the staging buffer's alignment pads it.
        let mut list = vec![0u8; HEADER_BYTES + 3 * ENTRY_BYTES + 8];
        list[..4].copy_from_slice(&3u32.to_ne_bytes());
        for (i, pixel) in [(5u32, 7u32), (9, 1)].into_iter().enumerate() {
            let at = HEADER_BYTES + i * ENTRY_BYTES + 32;
            list[at..at + 4].copy_from_slice(&(pixel.0 | pixel.1 << 16).to_ne_bytes());
        }
        let snapshot = Snapshot::new(&list, 2, |[x, y]| x * 100 + y);
        assert_eq!((snapshot.appended, snapshot.capacity), (3, 2));
        assert_eq!(snapshot.entries.len(), 2 * ENTRY_BYTES);
        assert_eq!(snapshot.epsilon, [507, 901]);
    }
}
