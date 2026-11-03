use super::*;

pub fn header(output: &mut impl Write) -> std::io::Result<()> {
    writeln!(
        output,
        "frame,phase,prep_us,encode_us,submit_us,projection_total_us,selection_total_us,admission_total_us,reads,uploads,geometry,peak,published,retiring,staging,pinned,payload,owned_buffers,metadata,scratch,cpu_ready,cut_clusters,candidates,visits,unresolved,max_error,candidate_debt,output_debt,residency_debt,selection_runs,io_queue_total_us,io_read_total_us,io_verify_total_us,failures,retries,evictions,retained_cuts,selection_calls,completion_total_us,resources_total_us,protection_total_us,stale_ready_total_us,make_room_total_us,upload_admission_total_us,requests_total_us,snapshot_total_us,dropped_ready,page_log_total_us"
    )
}

pub fn row(
    output: &mut impl Write,
    frame: u64,
    phase: &str,
    report: Option<&FrameReport>,
    s: &StreamStats,
    page_log_us: u128,
) -> std::io::Result<()> {
    writeln!(
        output,
        "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        frame,
        phase,
        report.map_or(0, |r| r.cpu.preparation.as_micros()),
        report.map_or(0, |r| r.cpu.encode.as_micros()),
        report.map_or(0, |r| r.cpu.submit.as_micros()),
        s.projection.as_micros(),
        s.selection.as_micros(),
        s.admission.as_micros(),
        s.reads,
        s.uploads,
        s.geometry_bytes,
        s.geometry_peak,
        s.published_bytes,
        s.retiring_bytes,
        s.staging_bytes,
        s.pinned_bytes,
        s.payload_bytes,
        s.owned_gpu_geometry_bytes,
        s.metadata_bytes,
        s.scratch_bound_bytes,
        s.cpu_ready_bytes,
        s.cut_clusters,
        s.candidates,
        s.cluster_visits,
        s.unresolved_groups,
        s.maximum_denied_error,
        s.candidate_debt,
        s.output_debt,
        s.residency_debt,
        s.selection_runs,
        s.io_queue.as_micros(),
        s.io_read.as_micros(),
        s.io_verify.as_micros(),
        s.failures,
        s.retries,
        s.evictions,
        s.retained_cuts,
        s.selection_calls,
        s.completion.as_micros(),
        s.resources.as_micros(),
        s.protection.as_micros(),
        s.stale_ready.as_micros(),
        s.make_room.as_micros(),
        s.upload_admission.as_micros(),
        s.requests.as_micros(),
        s.snapshot.as_micros(),
        s.dropped_ready,
        page_log_us
    )
}

/// Subtract cumulative work; current/peak resource observations retain their meaning.
pub fn since(total: &StreamStats, base: &StreamStats) -> StreamStats {
    let mut s = total.clone();
    s.projection -= base.projection;
    s.selection -= base.selection;
    s.admission -= base.admission;
    s.completion -= base.completion;
    s.resources -= base.resources;
    s.protection -= base.protection;
    s.stale_ready -= base.stale_ready;
    s.make_room -= base.make_room;
    s.upload_admission -= base.upload_admission;
    s.requests -= base.requests;
    s.snapshot -= base.snapshot;
    s.io_queue -= base.io_queue;
    s.io_read -= base.io_read;
    s.io_verify -= base.io_verify;
    s.dropped_ready -= base.dropped_ready;
    s.reads -= base.reads;
    s.uploads -= base.uploads;
    s.uploaded_bytes -= base.uploaded_bytes;
    s.inflight_retirements -= base.inflight_retirements;
    s.evictions -= base.evictions;
    s.retries -= base.retries;
    s.failures -= base.failures;
    s.selection_runs -= base.selection_runs;
    s.selection_calls -= base.selection_calls;
    s.retained_cuts -= base.retained_cuts;
    s
}
