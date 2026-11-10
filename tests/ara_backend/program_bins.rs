//! Semantic program-tile binning oracle, independent of atomic append ordering.
use std::collections::BTreeSet;

/// Clear-block marker; its coordinates name 16x16 blocks rather than 4x4 list tiles.
pub const CLEAR: u32 = 0x4000_0000;
/// Every slot-zero entry carries this flag, including hits for program zero.
pub const MISSES: u32 = 0x8000_0000;
const DISPATCH_ROW: u32 = 65_535;

/// Per-program semantic results; entry order within each GPU list is unspecified.
#[derive(Debug, PartialEq, Eq)]
pub struct Expected {
    /// Encoded tile entries per slot, with clear/miss flags preserved.
    pub sets: Vec<BTreeSet<u32>>,
    /// Number of entries, excluding the count header.
    pub counts: Vec<u32>,
    /// Indirect dispatch X/Y/Z and untouched pad word for every slot.
    pub args: Vec<[u32; 4]>,
}

/// Raw source-layout integer inputs for one complete original-kernel dispatch.
pub struct Fixture {
    /// RGBA32Uint hit texture dimensions.
    pub extent: [u32; 2],
    /// Row-major RGBA32Uint pixels; X is one plus the placement index, or zero.
    pub hit: Vec<[u32; 4]>,
    /// R32Uint coverage dimensions, one texel per 8x4 trace tile.
    pub coverage_extent: [u32; 2],
    /// Row-major coverage words; zero makes even nonzero hit records stale.
    pub coverage: Vec<u32>,
    /// Storage vec4u32: region X0/Y0/X1/Y1, widened to list-tile boundaries.
    pub region: [u32; 4],
    /// Storage u32 records: low 29 bits name a batch; high bits are placement flags.
    pub placements: Vec<u32>,
    /// RlList uniform: four header words then TRACED_DRAWS batch-to-slot words.
    pub list: [u32; 4 + renderlab::TRACED_DRAWS as usize],
    /// Allocated frame-program slots, including a hidden program with no live hits.
    pub slots: u32,
    /// Workgroup counts; the entry's local size is 4x4x1.
    pub groups: [u32; 3],
    /// Initial atomic entries: each slot has `list[2]` words, including its count.
    pub entries: Vec<u32>,
    /// Initial atomic dispatch words, cleared before running the original kernel.
    pub args: Vec<[u32; 4]>,
    /// Semantic expected lists, counts and indirect dispatch arguments.
    pub expected: Expected,
}

impl Fixture {
    /// Compare a readback without imposing an order on concurrent GPU appends.
    /// Unused list words and indirect pad words must retain their initial values.
    pub fn assert_matches(&self, entries: &[u32], args: &[[u32; 4]]) {
        assert_eq!(entries.len(), self.entries.len());
        assert_eq!(args, self.expected.args.as_slice());
        let capacity = self.list[2] as usize;
        for slot in 0..self.slots as usize {
            let base = slot * capacity;
            let count = entries[base] as usize;
            assert_eq!(
                count as u32, self.expected.counts[slot],
                "slot {slot} count"
            );
            assert!(count < capacity, "slot {slot} exceeded its list capacity");
            let actual: BTreeSet<_> = entries[base + 1..base + 1 + count]
                .iter()
                .copied()
                .collect();
            assert_eq!(actual.len(), count, "slot {slot} has duplicate entries");
            assert_eq!(actual, self.expected.sets[slot], "slot {slot} tile set");
            assert_eq!(
                entries[base + 1 + count..base + capacity],
                self.entries[base + 1 + count..base + capacity],
                "slot {slot} unused list words"
            );
        }
    }
}

/// Gather distinct visible programs per tile, then construct their dispatch shapes.
///
/// No workgroup mask or atomic sequence is simulated. Each region tile contributes
/// once to each program it contains; each image clear block contributes once to
/// slot zero. Fresh count headers and argument words are required, as in production.
pub fn reference(fixture: &Fixture) -> Expected {
    let [width, height] = fixture.extent;
    let [x0, y0, x1, y1] = fixture.region;
    assert!(x1 >= x0 && y1 >= y0 && width > 0 && height > 0);
    assert!(fixture.slots > 0 && fixture.slots <= 32);
    assert_eq!(fixture.hit.len(), (width * height) as usize);
    assert_eq!(
        fixture.coverage.len(),
        (fixture.coverage_extent[0] * fixture.coverage_extent[1]) as usize
    );
    assert!(fixture.coverage_extent[0] >= width.div_ceil(8));
    assert!(fixture.coverage_extent[1] >= height.div_ceil(4));
    let shaded = [(x1 - x0) / 4, (y1 - y0) / 4];
    let clear = [width.div_ceil(16), height.div_ceil(16)];
    assert!(fixture.groups[0] >= shaded[0].max(clear[0]));
    assert!(fixture.groups[1] >= shaded[1] + clear[1]);
    assert_eq!(fixture.groups[2], 1);
    assert_eq!(fixture.args, vec![[0; 4]; fixture.slots as usize]);
    let capacity = fixture.list[2];
    assert!(capacity > 0);
    assert_eq!(fixture.entries.len(), (fixture.slots * capacity) as usize);
    for slot in 0..fixture.slots {
        assert_eq!(fixture.entries[(slot * capacity) as usize], 0);
    }
    let mut sets = vec![BTreeSet::new(); fixture.slots as usize];
    for tile_y in 0..shaded[1] {
        for tile_x in 0..shaded[0] {
            let start = [x0 + tile_x * 4, y0 + tile_y * 4];
            let mut programs = BTreeSet::new();
            for y in start[1]..(start[1] + 4).min(height) {
                for x in start[0]..(start[0] + 4).min(width) {
                    let trace_cell = ((y / 4) * fixture.coverage_extent[0] + x / 8) as usize;
                    let placement = fixture.hit[(y * width + x) as usize][0];
                    let program = if fixture.coverage[trace_cell] == 0 || placement == 0 {
                        0
                    } else {
                        let batch = fixture.placements[(placement - 1) as usize] & 0x1fff_ffff;
                        assert!(batch < renderlab::TRACED_DRAWS);
                        fixture.list[4 + batch as usize]
                    };
                    assert!(program < fixture.slots);
                    programs.insert(program);
                }
            }
            let coordinate = tile_x | (tile_y << 15);
            for program in programs {
                sets[program as usize].insert(coordinate | if program == 0 { MISSES } else { 0 });
            }
        }
    }
    // Every image block is listed, including blocks wholly inside the region;
    // only the later primary_shade_list pass filters their pixels by the region.
    for y in 0..clear[1] {
        for x in 0..clear[0] {
            sets[0].insert(x | (y << 15) | CLEAR | MISSES);
        }
    }
    let counts: Vec<_> = sets.iter().map(|entries| entries.len() as u32).collect();
    assert!(counts.iter().all(|&count| count < capacity));
    let args = counts
        .iter()
        .map(|&count| {
            if count == 0 {
                [0; 4]
            } else {
                [count.min(DISPATCH_ROW), count.div_ceil(DISPATCH_ROW), 1, 0]
            }
        })
        .collect();
    Expected { sets, counts, args }
}

/// Mixed visible programs, uncovered stale hits, offset region and padded image edges.
/// Slot four is a real mapped program, but its hits are stale or outside the region.
pub fn fixture() -> Fixture {
    let extent = [23, 19];
    let coverage_extent = [3, 5];
    let coverage = vec![1, 1, 1, 1, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1];
    let region = [8, 4, 28, 24];
    let placements = vec![
        0x8000_0003,
        0x4000_0011,
        0xa000_0003,
        0x2000_002a,
        0xe000_003f,
        0x6000_0000,
    ];
    let mut list = [0; 4 + renderlab::TRACED_DRAWS as usize];
    // Count header + all 25 region tiles + all four clear blocks + three spare words.
    list[2] = 33;
    for (batch, program) in [(0, 0), (3, 1), (17, 2), (42, 3), (63, 4)] {
        list[4 + batch] = program;
    }
    let hit = (0..extent[0] * extent[1])
        .map(|pixel| {
            let x = pixel % extent[0];
            let y = pixel / extent[0];
            let trace = ((y / 4) * coverage_extent[0] + x / 8) as usize;
            let placement = if x < region[0] || y < region[1] || coverage[trace] == 0 {
                5 // A nonzero record for the hidden slot-four program.
            } else if (x + 2 * y) % 7 == 0 {
                0 // A genuine miss in a covered trace tile.
            } else {
                [1, 2, 3, 4, 6][((x + 3 * y) % 5) as usize]
            };
            [placement, 0x1357_0000 | pixel, 0x3e80_0000, 0x3f00_0000]
        })
        .collect();
    let slots = 5;
    let mut entries = vec![0xa5a5_f00d; (slots * list[2]) as usize];
    for slot in 0..slots {
        entries[(slot * list[2]) as usize] = 0;
    }
    let mut fixture = Fixture {
        extent,
        hit,
        coverage_extent,
        coverage,
        region,
        placements,
        list,
        slots,
        groups: [5, 7, 1],
        entries,
        args: vec![[0; 4]; slots as usize],
        expected: Expected {
            sets: vec![],
            counts: vec![],
            args: vec![],
        },
    };
    fixture.expected = reference(&fixture);
    fixture
}
