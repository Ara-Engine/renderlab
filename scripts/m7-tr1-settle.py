"""M7-TR1: settle equivalents on the physical Q-walk target, for TSR (TR0's bar rows) and
candidates (m7-tr1-score.py JSON), since TR0's frozen Exact/film bar no longer exists.

  py -3 -B scripts/m7-tr1-settle.py --case w-e-r-c1 name=score.json [...]

Stop settle: the camera stops at frame 64 and holds through 96 (every hold frame has a
reference). For each engine and population, the first hold frame from which the error stays
within 2% (and 0.1 code) of that engine's own frame-96 value, counted from the stop; plus the
error at +8 (frame 72) against TSR's. Trail width: the largest p95 / maximum nearest-edge
distance of background-centre pixels whose error exceeds 1, 4 or 8 codes, over all frames
(TR0's thresholded extent; zero pixels means no trail beyond 1.5 px).
"""
import argparse
import csv
import importlib.util
import json
from pathlib import Path

spec = importlib.util.spec_from_file_location("score", Path(__file__).with_name("m7-tr1-score.py"))
score_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(score_module)
# Every hold pixel is old; TR0 has age masks only at the 60 T1b checkpoints, so settle uses
# the unaged populations.
POPULATIONS = ["all-covered", "partial", "edge", "occluded", "background-centre"]


def settle(rows, population, channel):
    series = {f: rows.get((str(f), channel, population, "rms_noise_subtracted")) for f in range(64, 97)}
    if any(v is None for v in series.values()):
        return None
    final = series[96]
    bound = max(final * 1.02, final + 0.1)
    first = 96
    for f in range(96, 63, -1):
        if series[f] > bound:
            break
        first = f
    return dict(frames_after_stop=first - 64, frame_72=series[72], frame_96=final)


def trails(rows):
    out = {}
    for threshold in ("1", "4", "8"):
        for channel in ("L0", "L4"):
            pixels = [v for (f, c, p, m), v in rows.items() if p == "background-error-over-" + threshold
                      and c == channel and m == "pixels"]
            p95 = [v for (f, c, p, m), v in rows.items() if p == "background-error-over-" + threshold
                   and c == channel and m == "p95_distance_px"]
            mx = [v for (f, c, p, m), v in rows.items() if p == "background-error-over-" + threshold
                  and c == channel and m == "maximum_distance_px"]
            out[f"{channel}>{threshold}"] = dict(frames=len(pixels), max_pixels=max(pixels, default=None),
                                                 max_p95_px=max(p95, default=None), max_distance_px=max(mx, default=None))
    return out


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--case", required=True)
    parser.add_argument("candidates", nargs="+")
    args = parser.parse_args()
    tsr = {}
    with (score_module.TR0 / "bar/physical-measures.csv").open(newline="", encoding="utf-8") as stream:
        for r in csv.DictReader(stream):
            if r["label"] == args.case and r["method"] == "tsr":
                tsr[(r["frame"], r["channel"], r["population"], r["measure"])] = float(r["value"])
    engines = {"tsr": tsr}
    for entry in args.candidates:
        name, path = entry.split("=", 1)
        engines[name] = {k: v[0] for k, v in score_module.rows_of(json.loads(Path(path).read_text())).items()}
    result = {}
    for name, rows in engines.items():
        result[name] = dict(settle={f"{p}|{c}": settle(rows, p, c) for p in POPULATIONS for c in ("L0", "L4")},
                            trails=trails(rows))
    print(json.dumps(result, indent=1))


if __name__ == "__main__":
    main()
