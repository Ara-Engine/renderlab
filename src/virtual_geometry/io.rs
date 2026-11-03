use super::*;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};

pub(super) struct ReadResult {
    pub page: PageId,
    pub geometry: Result<PageGeometry, String>,
    pub queue: Duration,
    pub read: Duration,
    pub verify: Duration,
}
struct Request {
    page: PageId,
    record: Page,
    queued: Instant,
}
/// Exactly four outstanding requests including active IO and completed CPU payloads.
/// Drop disconnects both channels before joining; neither send nor delay can trap shutdown.
pub(super) struct PageIo {
    requests: Option<mpsc::SyncSender<Request>>,
    results: Option<mpsc::Receiver<ReadResult>>,
    worker: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}
impl PageIo {
    pub fn new(directory: PathBuf, delay: Duration) -> Self {
        let (tx, rx) = mpsc::sync_channel::<Request>(READ_SLOTS);
        let (results, completed) = mpsc::sync_channel(READ_SLOTS);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let worker = thread::Builder::new()
            .name("renderlab-pages".into())
            .spawn(move || {
                while let Ok(request) = rx.recv() {
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    let queue = request.queued.elapsed();
                    let start = Instant::now();
                    let mut bytes = vec![0; PAGE_BYTES + 1];
                    let read = (|| {
                        let mut file = File::open(directory.join(hex(&request.record.sha256)))?;
                        let mut count = 0;
                        while count < bytes.len() {
                            let n = file.read(&mut bytes[count..])?;
                            if n == 0 {
                                break;
                            }
                            count += n;
                        }
                        bytes.truncate(count);
                        Ok::<_, std::io::Error>(())
                    })();
                    let read_time = start.elapsed();
                    let start = Instant::now();
                    let geometry = read
                        .map_err(|e| format!("page {} IO: {e}", request.page.0))
                        .and_then(|()| {
                            let hash: [u8; 32] = Sha256::digest(&bytes).into();
                            decode_page(&request.record, &bytes, hash).map_err(|e| e.to_string())
                        });
                    let verify = start.elapsed();
                    drop(bytes);
                    // Delay actual completion, after real read/hash/decode, on the worker.
                    let until = Instant::now() + delay;
                    while !stopping.load(Ordering::Acquire) && Instant::now() < until {
                        thread::park_timeout(until.saturating_duration_since(Instant::now()));
                    }
                    if stopping.load(Ordering::Acquire) {
                        break;
                    }
                    if results
                        .send(ReadResult {
                            page: request.page,
                            geometry,
                            queue,
                            read: read_time,
                            verify,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .expect("page worker thread creation");
        Self {
            requests: Some(tx),
            results: Some(completed),
            worker: Some(worker),
            stop,
        }
    }
    pub fn request(&self, page: PageId, record: &Page) -> bool {
        self.requests
            .as_ref()
            .unwrap()
            .try_send(Request {
                page,
                record: record.clone(),
                queued: Instant::now(),
            })
            .is_ok()
    }
    pub fn receive(&self) -> Option<ReadResult> {
        self.results.as_ref().unwrap().try_recv().ok()
    }
}
impl Drop for PageIo {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.results.take();
        self.requests.take();
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}
pub(super) fn hex(hash: &[u8; 32]) -> String {
    hash.iter().map(|v| format!("{v:02x}")).collect()
}

/// Bounded synchronous metadata bootstrap. Never reads or scans page blobs.
pub(super) fn load(cache: &Path, manifest: &Path) -> Result<Package, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    File::open(manifest)?.take(81).read_to_end(&mut bytes)?;
    if bytes.len() != 80 || &bytes[..8] != b"RVM\0\x01\0\0\0" {
        return Err("invalid virtual manifest".into());
    }
    let digest: [u8; 32] = bytes[40..72].try_into()?;
    let length = u64::from_le_bytes(bytes[72..80].try_into()?);
    if length > (METADATA_LIMIT + TEXTURE_LIMIT) as u64 {
        return Err("metadata bootstrap file cap".into());
    }
    let mut file = File::open(cache.join("virtual-v1/packages").join(hex(&digest)))?;
    let mut header = [0u8; 24];
    file.read_exact(&mut header)?;
    let resources = u32::from_le_bytes(header[20..24].try_into()?) as usize;
    // Reserve the encoded metadata AND decoded/validation element storage together.
    // One MiB covers bounded M3 material/texture/mip containers (texels are separate).
    let encoded_metadata = (length as usize)
        .checked_sub(resources)
        .filter(|n| *n >= 24)
        .ok_or("invalid resource extent")?;
    if resources > TEXTURE_LIMIT || encoded_metadata >= METADATA_LIMIT - (1 << 20) {
        return Err("metadata/resource bootstrap capacity".into());
    }
    let limits = VirtualLimits {
        max_metadata_bytes: METADATA_LIMIT - (1 << 20) - encoded_metadata,
        prepared: asset::Limits {
            max_decoded_bytes: TEXTURE_LIMIT,
            ..Default::default()
        },
        ..Default::default()
    };
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::with_capacity(length as usize + 1);
    file.take(length + 1).read_to_end(&mut bytes)?;
    if bytes.len() != length as usize || <[u8; 32]>::from(Sha256::digest(&bytes)) != digest {
        return Err("package extent/digest mismatch".into());
    }
    Ok(asset::virtual_mesh::decode(&bytes, limits)?)
}
