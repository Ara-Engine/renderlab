//! CPU counters and bounded pass-log marks; no shaders or GPU resources.
use crate::pass_timeline::PassTimeline;
use std::{
    fs::File,
    io::{BufWriter, Write},
    sync::{Mutex, OnceLock},
};

#[derive(Default)]
pub(super) struct Counts {
    pub draws: u64,
    pub emitters: u64,
    pub particle_capacity: u64,
    pub particle_updates: u64,
    pub particle_sorts: u64,
    pub copies: u64,
    pub copied_pixels: u64,
    pub lobe_passes: u64,
    pub nonreflective_lobe_runs: u64,
    pub lobe_target_pixels: u64,
    pub reflection_dispatches: u64,
    pub reflection_pixels: u64,
    pub pane_ids: u64,
    pub pane_shades: u64,
    pub pane_rect_pixels: u64,
    pub raster_passes: u64,
    pub msaa_passes: u64,
    pub seed_pixels: u64,
    pub resolve_pixels: u64,
}

pub(super) fn area(rect: [u32; 4]) -> u64 {
    u64::from(rect[2].saturating_sub(rect[0])) * u64::from(rect[3].saturating_sub(rect[1]))
}

fn log() -> Option<&'static Mutex<BufWriter<File>>> {
    static LOG: OnceLock<Option<Mutex<BufWriter<File>>>> = OnceLock::new();
    LOG.get_or_init(|| {
        let path = std::env::var_os("RENDERLAB_TRANSPARENCY_LOG").filter(|p| !p.is_empty())?;
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            Ok(file) => Some(Mutex::new(BufWriter::new(file))),
            Err(error) => {
                eprintln!("transparency measurement log: {error}");
                None
            }
        }
    })
    .as_ref()
}

fn part_label(budget: usize, name: &'static str) -> Option<&'static str> {
    match budget {
        0..=3 => None,
        4 => Some("transparency.remainder"),
        _ => Some(name),
    }
}

pub(super) struct Measure {
    pub counts: Counts,
    frame: u64,
    view: u64,
    marks: u32,
    incomplete: bool,
    remainder: bool,
}

impl Measure {
    pub fn begin(frame: u64, view: u64) -> Option<Self> {
        log()?;
        Some(Self {
            counts: Counts::default(),
            frame,
            view,
            marks: 0,
            incomplete: false,
            remainder: false,
        })
    }

    /// Leave room for ids, post, and the final timestamp. At exhaustion, explicitly
    /// measure the remaining transparency work as one unclassified bucket.
    pub fn part(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        timeline: &mut Option<&mut PassTimeline>,
        name: &'static str,
    ) {
        if self.remainder {
            return;
        }
        let Some(timeline) = timeline.as_deref_mut() else {
            self.incomplete = true;
            return;
        };
        let budget = timeline.detail_marks_remaining();
        if let Some(label) = part_label(budget, name) {
            timeline.mark(encoder, label);
            self.marks += 1;
        }
        if budget <= 4 {
            self.incomplete = true;
            self.remainder = true;
        }
    }

    fn write(
        &self,
        writer: &mut impl Write,
        rt: bool,
        size: [u32; 2],
        shadow: bool,
    ) -> std::io::Result<()> {
        write!(
            writer,
            "{{\"frame_zero_based\":{},\"view\":{},\"rt\":{rt},\"size\":[{},{}],\"shadow_map_rendered\":{shadow},\"detail_marks\":{},\"detail_complete\":{}",
            self.frame, self.view, size[0], size[1], self.marks, !self.incomplete
        )?;
        let c = &self.counts;
        for (name, value) in [
            ("draws", c.draws),
            ("emitters", c.emitters),
            ("particle_capacity", c.particle_capacity),
            ("particle_updates", c.particle_updates),
            ("particle_sorts", c.particle_sorts),
            ("copies", c.copies),
            ("copied_pixels", c.copied_pixels),
            ("lobe_passes", c.lobe_passes),
            ("nonreflective_lobe_runs", c.nonreflective_lobe_runs),
            ("lobe_target_pixels", c.lobe_target_pixels),
            ("reflection_dispatches", c.reflection_dispatches),
            ("reflection_pixels", c.reflection_pixels),
            ("pane_ids", c.pane_ids),
            ("pane_shades", c.pane_shades),
            ("pane_rect_pixels", c.pane_rect_pixels),
            ("raster_passes", c.raster_passes),
            ("msaa_passes", c.msaa_passes),
            ("seed_pixels", c.seed_pixels),
            ("resolve_pixels", c.resolve_pixels),
        ] {
            write!(writer, ",\"{name}\":{value}")?;
        }
        writeln!(writer, "}}")
    }

    pub fn finish(self, rt: bool, size: [u32; 2], shadow: bool) {
        if let Some(log) = log() {
            if let Ok(mut writer) = log.lock() {
                if let Err(error) = self
                    .write(&mut *writer, rt, size, shadow)
                    .and_then(|_| writer.flush())
                {
                    eprintln!("transparency measurement log: {error}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn measurement_records_area_and_flags_incomplete_detail() {
        assert_eq!(
            part_label(5, "transparency.copy"),
            Some("transparency.copy")
        );
        assert_eq!(
            part_label(4, "transparency.copy"),
            Some("transparency.remainder")
        );
        assert_eq!(part_label(3, "transparency.copy"), None);
        assert_eq!(area([2, 3, 12, 23]), 200);
        assert_eq!(area([12, 3, 2, 23]), 0);
        let mut m = Measure {
            counts: Counts::default(),
            frame: 30,
            view: 4,
            marks: 0,
            incomplete: true,
            remainder: true,
        };
        m.counts.copied_pixels = 200;
        m.counts.nonreflective_lobe_runs = 2;
        let mut text = Vec::new();
        m.write(&mut text, true, [1920, 1080], false).unwrap();
        let row: serde_json::Value = serde_json::from_slice(&text).unwrap();
        assert_eq!(row["frame_zero_based"], 30);
        assert_eq!(row["copied_pixels"], 200);
        assert_eq!(row["nonreflective_lobe_runs"], 2);
        assert_eq!(row["detail_complete"], false);
    }
}
