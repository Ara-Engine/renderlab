"""U-SPECK: the GI cache's per-frame counters (RENDERLAB_GI_LOG), summarized per run.

    py -3 -B tools/speck/gilog.py RUN_DIR [RUN_DIR ...]      # each holds gi.jsonl

Prints the table's size, totals over the run and a few frames' counters: cells created, failed
inserts (a full bucket, counted once per claim round; and the pixels' share of them), cells a full bucket gave up before
their eviction age and those of them seen within the last eight frames (cells the view may
still be reading: thrash), listed misses, gathers and the gather queue.
"""
import json
import sys
from pathlib import Path


def main():
    for run in sys.argv[1:]:
        rows = [json.loads(line) for line in (Path(run) / "gi.jsonl").read_text().splitlines() if line.strip()]
        total = lambda key: sum(r.get(key, 0) for r in rows)
        pick = [i for i in (0, 1, 10, 30, 60, 90, len(rows) - 1) if i < len(rows)]
        print(f"{Path(run).name}: {len(rows)} frames, table {rows[-1]['table_bytes'] >> 20} MiB | created {total('created')}, failed inserts {total('full')} (pixels' {total('full_pixel')}), "
              f"evicted under pressure {total('evicted')} (seen within 8 frames: {total('evicted_recent')}), lost {total('lost')}, orphans {total('orphans')}")
        print("   frame: created / failed / pixels' failed / evicted / evicted-recent / misses / gathers / gather queue")
        for i in pick:
            r = rows[i]
            print(f"   {i:4d}: {r['created']} / {r['full']} / {r.get('full_pixel', 0)} / {r.get('evicted', 0)} / {r.get('evicted_recent', 0)} / {r['misses']} / {r['gathers']} / {r['bounce_queued']}")


if __name__ == "__main__":
    main()
