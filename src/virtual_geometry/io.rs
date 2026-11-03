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
/// Drop disconnects both channels before joining; neither send nor delay can trap shutdown.
pub(super) struct PageIo {
    requests: Option<mpsc::SyncSender<Request>>,
    results: Option<mpsc::Receiver<ReadResult>>,
    worker: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}
impl PageIo {
    pub fn metadata_bytes(slots: usize) -> Result<usize, Error> {
        slots
            .checked_mul(std::mem::size_of::<Request>() + std::mem::size_of::<ReadResult>())
            .ok_or(Error::InvalidInput("read_slots metadata overflow"))
    }
    pub fn new(directory: PathBuf, delay: Duration, slots: usize) -> Result<Self, Error> {
        let (tx, rx) = mpsc::sync_channel::<Request>(slots);
        let (results, completed) = mpsc::sync_channel(slots);
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
                    let until = Instant::now()
                        .checked_add(delay)
                        .unwrap_or_else(Instant::now);
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
            .map_err(|_| Error::InvalidInput("page worker thread creation failed"))?;
        Ok(Self {
            requests: Some(tx),
            results: Some(completed),
            worker: Some(worker),
            stop,
        })
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
pub(super) fn load(
    cache: &Path,
    manifest: &Path,
    options: StreamOptions,
) -> Result<Package, Box<dyn std::error::Error>> {
    #[cfg(feature = "selection-profile")]
    let profile = Instant::now();
    options.validate()?;
    let mut bytes = Vec::new();
    File::open(manifest)?.take(81).read_to_end(&mut bytes)?;
    if bytes.len() != 80 || &bytes[..8] != b"RVM\0\x01\0\0\0" {
        return Err("invalid virtual manifest".into());
    }
    let digest: [u8; 32] = bytes[40..72].try_into()?;
    let length = u64::from_le_bytes(bytes[72..80].try_into()?);
    if length > options.bootstrap_bytes as u64
        || length > options.limits.prepared.max_file_bytes as u64
    {
        return Err("metadata bootstrap file cap".into());
    }
    let mut file = File::open(cache.join("virtual-v1/packages").join(hex(&digest)))?;
    let mut header = [0u8; 24];
    file.read_exact(&mut header)?;
    let resources = u32::from_le_bytes(header[20..24].try_into()?) as usize;
    // Reserve the encoded metadata AND decoded/validation element storage together.
    let length = usize::try_from(length).map_err(|_| "package exceeds host address space")?;
    let encoded_metadata = length
        .checked_sub(resources)
        .filter(|n| *n >= 24)
        .ok_or("invalid resource extent")?;
    if resources > options.texture_bytes || encoded_metadata >= options.metadata_bytes {
        return Err("metadata/resource bootstrap capacity".into());
    }
    let mut limits = options.limits;
    limits.max_metadata_bytes = limits
        .max_metadata_bytes
        .min(options.metadata_bytes - encoded_metadata);
    limits.prepared.max_decoded_bytes = limits.prepared.max_decoded_bytes.min(
        options
            .texture_bytes
            .checked_add(options.metadata_bytes - encoded_metadata)
            .ok_or("bootstrap decode byte overflow")?,
    );
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| "package allocation failed")?;
    file.take((length as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    #[cfg(feature = "selection-profile")]
    let read_us = profile.elapsed().as_micros();
    if bytes.len() != length || <[u8; 32]>::from(Sha256::digest(&bytes)) != digest {
        return Err("package extent/digest mismatch".into());
    }
    #[cfg(feature = "selection-profile")]
    let verified_us = profile.elapsed().as_micros();
    let package = asset::virtual_mesh::decode(&bytes, limits)?;
    #[cfg(feature = "selection-profile")]
    eprintln!(
        "bootstrap_profile bytes={length} read_us={read_us} digest_us={} decode_us={}",
        verified_us - read_us,
        profile.elapsed().as_micros() - verified_us
    );
    Ok(package)
}
