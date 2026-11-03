use std::{
    cell::Cell,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

/// Opt-in host-call boundaries. An unmatched begin means no return was observed.
pub struct SurfaceProbe {
    origin: Option<Instant>,
    next_id: Cell<u64>,
}

pub struct Span {
    timing: Option<(Instant, Instant)>,
    id: u64,
    phase: &'static str,
}

impl SurfaceProbe {
    pub fn new(enabled: bool) -> Self {
        let origin = enabled.then(Instant::now);
        if origin.is_some() {
            eprintln!(
                "surface_probe version=1 pid={} epoch_us={}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_micros()
            );
        }
        Self {
            origin,
            next_id: Cell::new(0),
        }
    }

    pub fn event(&self, event: &'static str) {
        if let Some(origin) = self.origin {
            eprintln!(
                "surface_probe event={event} at_us={}",
                origin.elapsed().as_micros()
            );
        }
    }

    pub fn begin(&self, phase: &'static str) -> Span {
        let id = self.next_id.get();
        let timing = self.origin.map(|origin| {
            self.next_id.set(id + 1);
            eprintln!(
                "surface_probe begin={id} phase={phase} at_us={}",
                origin.elapsed().as_micros()
            );
            (origin, Instant::now())
        });
        Span { timing, id, phase }
    }
}

impl Span {
    pub fn end(self, outcome: &'static str) {
        if let Some((origin, start)) = self.timing {
            let end = Instant::now();
            eprintln!(
                "surface_probe end={} phase={} at_us={} duration_us={} outcome={outcome}",
                self.id,
                self.phase,
                end.duration_since(origin).as_micros(),
                end.duration_since(start).as_micros()
            );
        }
    }
}
