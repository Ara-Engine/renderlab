"""CPU summary of the optimized M1 RT1c quiet comparison; never starts GPU work."""
import argparse
import csv
import json
from pathlib import Path
import statistics


def summary(values):
    return dict(median=statistics.median(values), minimum=min(values), maximum=max(values))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    args = parser.parse_args()
    result, visits = {}, []
    for name in ('rt', 'rt-raster-exact'):
        directory = args.root/'m1'/name
        data = json.loads((directory/'m1-cost.json').read_text())
        rows = data['samples']
        assert len(rows) == 1920 and data['visits_per_pose'] == 8
        phases = {}
        for phase, expected in (('static',8),('camera',48),('sun',16),('caster',48)):
            groups = []
            for pose in (0,1):
                for visit in range(8):
                    sample = [r for r in rows if (r['phase'],r['pose'],r['visit']) == (phase,pose,visit)]
                    assert len(sample) == expected
                    medians = {key: statistics.median(r[key] for r in sample) if sample[0][key] is not None else None
                               for key in ('primary_ms','shadow_ms','shade_ms','tone_ms','span_ms')}
                    row = dict(path=name,phase=phase,pose=pose,visit=visit,**medians)
                    row['legacy_rt_clean'] = medians['primary_ms'] <= .6 and medians['shade_ms'] <= .5 if name == 'rt' else None
                    groups.append(row)
                    visits.append(row)
            phase_data = {key:summary([r[key] for r in groups]) if groups[0][key] is not None else None
                          for key in ('primary_ms','shadow_ms','shade_ms','tone_ms','span_ms')}
            phase_data['legacy_rt_clean_visits'] = sum(r['legacy_rt_clean'] for r in groups) if name == 'rt' else None
            phase_data['visits'] = len(groups)
            phases[phase] = phase_data
        reports = [json.loads(p.read_text()) for p in directory.glob('memory-p*.json')]
        assert len(reports) == 16
        memory = {key:summary([r[key]/2**20 for r in reports])
                  for key in ('allocator_allocated_bytes','allocator_reserved_bytes')}
        run = json.loads((directory/'run.json').read_text())
        memory.update(wddm_warm_mib=run['wddm_warm_max_bytes']/2**20,
                      wddm_process_mib=run['wddm_max_bytes']/2**20,
                      wddm_samples=run['wddm_samples'],wddm_warm_samples=run['wddm_warm_samples'])
        result[name] = dict(primary=data['primary'],shadows=data['shadows'],phases=phases,memory=memory)
    result['recorded_hrt_ms'] = {'static':[1.743,1.753],'camera':[1.764,1.766],'sun':[1.738,1.751],'caster':[1.776,1.778]}
    result['ratios'] = {}
    for phase,bounds in result['recorded_hrt_ms'].items():
        fast=result['rt']['phases'][phase]['span_ms'];default=result['rt-raster-exact']['phases'][phase]['span_ms']
        result['ratios'][phase]=dict(default_over_fast=default['median']/fast['median'],
            hrt_over_fast_median_bounds=[x/fast['median'] for x in bounds],
            hrt_over_fast_visit_bounds=[bounds[0]/fast['maximum'],bounds[1]/fast['minimum']])
    result['scope']='Medians of visit medians; ranges retain all 16 visits. Legacy RT clean screen: primary <= .60 ms and shade <= .50 ms; not applicable to full raster geometry costs. HRT is recorded, not paired or rerun.'
    (args.root/'summary.json').write_text(json.dumps(result,indent=2))
    with (args.root/'visits.csv').open('w',newline='') as stream:
        writer=csv.DictWriter(stream,fieldnames=list(visits[0]));writer.writeheader();writer.writerows(visits)
    print(json.dumps(result,indent=2))


if __name__ == '__main__':
    main()
