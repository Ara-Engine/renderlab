"""Audit saved EXR bytes, denominator conditioning and local UE source identities.

No rendering or fitted corrections. Requires the same private OpenEXR dependency
directory as ue-quarry-extract.py. Output belongs in an ignored evidence directory.
"""
import argparse
import importlib.util
import json
from pathlib import Path
import sys

import numpy as np


def load(path):
    spec=importlib.util.spec_from_file_location(path.stem.replace('-','_'),path)
    m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
    return m


def audit(output):
    diag=load(Path(__file__).with_name('ue-shadow-diagnose.py'))
    extract=load(Path(__file__).with_name('ue-quarry-extract.py'))
    sys.path.insert(0,str(extract.DEPENDENCIES))
    import OpenEXR
    results=dict(exrs=[],conditioning=[],sources={})
    for mode in ('vsm','hrt'):
        run=diag.ROOT/'runs'/f'full-{mode}-capture-01'
        capture=json.loads((run/'capture.json').read_text())
        raw=np.load(diag.ROOT/'extracted'/run.name/'pose-0-steady-direct.npz')
        for shadows in (False,True):
            rec=next(x for x in capture['captures'] if x['pose']==0 and x['state']=='steady' and x['casts_shadows']==shadows)
            path=run/rec['path'];sha=diag.digest(path)
            assert sha==rec['sha256']
            with OpenEXR.File(str(path),separate_channels=True) as exr:
                rgb=np.stack([exr.channels()[c].pixels for c in 'RGB'],-1)
                assert np.array_equal(rgb,raw['shadowed' if shadows else 'unshadowed'])
                results['exrs'].append(dict(path=str(path),sha256=sha,dtype=str(rgb.dtype),
                    nonfinite=int((~np.isfinite(rgb)).sum()),negative=int((rgb<0).sum()),
                    at_fp16_limit=int((rgb>=65504).sum()),maximum=float(rgb.max()),
                    float16_roundtrip_equal=bool(np.array_equal(rgb,rgb.astype('f2').astype('f4'))),
                    matches_extracted_array=True))
        for pose in range(3):
            p=diag.ROOT/'extracted'/run.name/f'pose-{pose}-steady-direct.npz'
            with np.load(p) as a, np.load(diag.REFERENCE/f'pose-{pose}-reference.npz') as b:
                v,valid,lum,ref,cover=a['visibility'],a['valid'],a['unshadowed']@[.2126,.7152,.0722],b['ref'],b['cover']
                partial=cover&(ref>0)&(ref<1)
                thresholds=[]
                for minimum in [1e-6,.001,.1,1,5]:
                    sel=partial&valid&(lum>minimum)
                    thresholds.append(dict(minimum_direct=minimum,error=diag.stats((v-ref)[sel])))
                solids=[]
                for label,pop in [('lit',ref==1),('umbra',ref==0)]:
                    sel=cover&valid&pop
                    solids.append(dict(population=label,
                        counts={str(t):int((sel&(abs(v-ref)>t)).sum()) for t in [.002,.01,.5,.95]}))
                results['conditioning'].append(dict(mode=mode,pose=pose,thresholds=thresholds,solids=solids))
    engine=Path('C:/Program Files/Epic Games/UE_5.7/Engine')
    files=['Source/Runtime/Engine/Private/Components/DirectionalLightComponent.cpp',
           'Source/Runtime/Renderer/Private/VirtualShadowMaps/VirtualShadowMapArray.cpp',
           'Source/Runtime/Renderer/Private/RayTracing/RayTracingShadows.cpp',
           'Source/Runtime/Renderer/Private/ScreenSpaceDenoise.cpp',
           'Shaders/Private/RayTracing/RayTracingCommon.ush','Shaders/Private/RayTracing/RayTracingOcclusionRGS.usf',
           'Shaders/Private/RayTracing/RayTracingDirectionalLight.ush',
           'Shaders/Private/DeferredLightingCommon.ush','Shaders/Private/ShadingModels.ush',
           'Shaders/Private/ScreenSpaceDenoise/SSDTemporalAccumulation.usf',
           'Shaders/Private/VirtualShadowMaps/VirtualShadowMapProjectionCommon.ush',
           'Shaders/Private/VirtualShadowMaps/VirtualShadowMapProjectionDirectional.ush',
           'Shaders/Private/VirtualShadowMaps/VirtualShadowMapProjection.usf',
           'Shaders/Private/VirtualShadowMaps/VirtualShadowMapScreenRayTrace.ush']
    for name in files:results['sources'][name]=diag.digest(engine/name)
    output.write_text(json.dumps(results,indent=2,allow_nan=False)+'\n')
    print(json.dumps(results,indent=2))


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output',type=Path,required=True)
    audit(p.parse_args().output)
