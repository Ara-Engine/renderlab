"""Reuse a reference only when physical/display inputs match exactly.

GI and reflection enable flags select renderer estimators, not a different physical
target. Everything else (including medium, alpha, post, lights and every pose) is
compared. The source identity and process record remain explicit in each sidecar.
"""
import copy
import argparse
import json
from pathlib import Path
import shutil

from capture import digest
from prepare import ROOT, save


def physical_inputs(scene):
    value=copy.deepcopy(scene)
    value.pop('name',None);value.pop('notes',None)
    effects={'gi':True,'reflections':True,'local_shadows':True,'atmosphere':False,'fog':False,'post':False,'alpha_panel':False}
    effects.update(value.get('effects',{}))
    effects.pop('gi');effects.pop('reflections')
    value['effects']=effects
    return value


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source-tag',default='references-hq-final')
    p.add_argument('--tag',default='references-reused-final')
    a=p.parse_args()
    source=ROOT/'phase2-scenes/game-base-quality.json'
    source_spec=json.loads(source.read_text())
    folder=ROOT/'phase2-runs'/a.source_tag/'game-base-quality/reference'
    process=json.loads((folder/'process.json').read_text())
    if process['exit_code'] or not process['source_status_unchanged'] or process['scene_sha256']!=digest(source) or process['asset_sha256']!=digest(Path(source_spec['asset'])):
        raise ValueError('reference input changed or did not complete')
    for variant in ('no-gi','no-reflections'):
        target=ROOT/'phase2-scenes'/f'game-{variant}-quality.json'
        if physical_inputs(source_spec)!=physical_inputs(json.loads(target.read_text())):
            raise ValueError('reference targets are not physically identical')
        out=ROOT/'phase2-runs'/a.tag/f'game-{variant}-quality/reference'
        if out.exists():raise ValueError('reference aliases already exist')
        (out/'reference-halves').mkdir(parents=True)
        for image in folder.glob('frame-*.png'):
            shutil.copy2(image,out/image.name)
            value=json.loads(image.with_suffix('.png.json').read_text())
            value.update(scene_sha256=digest(target),reference_source_scene_sha256=digest(source),
                reference_source_image=str(image),reference_source_process=str(folder/'process.json'),
                reuse_qualification='exact physical/display input identity, excluding only documentary name/notes and renderer GI/reflection estimator switches')
            save(out/(image.name+'.json'),value)
        for image in (folder/'reference-halves').glob('*.png'):shutil.copy2(image,out/'reference-halves'/image.name)
        print(out)


if __name__=='__main__':main()
