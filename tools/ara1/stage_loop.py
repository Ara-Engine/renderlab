"""Prepare private adapter copies for real Cargo edit/rebuild/readback proxies.

Only our adapter source is copied/edited. Ara and Renderlab renderer source stays
read-only through absolute library/dependency paths. The result is explicitly an
adapter-edit proxy, not a core-renderer edit benchmark.
"""
import json
from pathlib import Path
import re
import shutil

from capture import REPO
from prepare import ROOT, save


def main():
    stage=ROOT/'staged';stage.mkdir(exist_ok=True)
    scene=json.loads((ROOT/'phase2-scenes/game-base-quality.json').read_text())
    scene['frames']=scene['frames'][:1];scene['frames'][0]['capture']=True
    save(stage/'game.json',scene)
    for engine in ('renderlab','ara'):
        project=stage/engine;project.mkdir(exist_ok=True)
        if engine=='renderlab':
            source=(REPO/'examples/ara1_capture.rs').read_text()
            source=source.replace('../tools/ara1/scene.rs',(REPO/'tools/ara1/scene.rs').as_posix())
            source=source.replace('include_str!("game_scene.wgsl")',f'include_str!("{(REPO/"examples/game_scene.wgsl").as_posix()}")')
            source=source.replace('../shaders/programs/standard.wgsl',(REPO/'shaders/programs/standard.wgsl').as_posix())
            manifest=(REPO/'Cargo.toml').read_text().replace('members = ["viewer", "crates/asset", "crates/cook"]','members = []')
            manifest=manifest.replace('path = "crates/asset"',f'path = "{(REPO/"crates/asset").as_posix()}"')
            manifest=manifest.replace('path = "crates/cook"',f'path = "{(REPO/"crates/cook").as_posix()}"')
            manifest += f'\n[lib]\npath = "{(REPO/"src/lib.rs").as_posix()}"\n[[example]]\nname = "ara1_capture"\npath = "{(project/"main.rs").as_posix()}"\n'
            shutil.copyfile(REPO/'Cargo.lock',project/'Cargo.lock')
            target=ROOT/'target-renderlab'
            build=['cargo','build','--manifest-path',str(project/'Cargo.toml'),'--release','--example','ara1_capture','-j','1']
            binary=target/'release/examples/ara1_capture.exe'
            find='stops: spec.exposure_stops,';replace='stops: spec.exposure_stops + 0.125,'
        else:
            original=ROOT/'adapter-ara-main'
            source=(REPO/'tools/ara1/ara.rs').read_text()
            for module in ('scene','ara_textures'):
                source=source.replace(f'mod {module};',f'#[path = "{(REPO/f"tools/ara1/{module}.rs").as_posix()}"]\nmod {module};')
            manifest=(original/'Cargo.toml').read_text().replace((REPO/'tools/ara1/ara.rs').as_posix(),(project/'main.rs').as_posix())
            for name in ('Cargo.lock','build.rs','sample.rs'):shutil.copyfile(original/name,project/name)
            target=ROOT/'target-ara-main'
            build=['cargo','+1.99.1','build','--manifest-path',str(project/'Cargo.toml'),'--release','-j','1']
            binary=target/'release/ara1-ara.exe'
            find='2.0_f32.powf(spec.exposure_stops)';replace='2.0_f32.powf(spec.exposure_stops + 0.125)'
        (project/'main.rs').write_text(source)
        (project/'Cargo.toml').write_text(manifest)
        out=ROOT/'devloops'/engine
        capture=out/'capture'
        command=['py','-3','-B',str(REPO/f'tools/ara1/run-{engine}.py'),'--binary',str(binary),
            '--source',str(REPO if engine=='renderlab' else ROOT/'ara-main'),'--scene',str(stage/'game.json'),
            '--mode','nonrt' if engine=='renderlab' else 'raster','--out',str(capture),
            '--profile','release','--contention','shared CPU; one renderer; real staged adapter Rust edit and rebuild; not core renderer edit']
        if engine=='ara':command += ['--bake-cache',str(ROOT/'bakes/p2-game-normal')]
        save(project/'recipe.json',dict(engine=engine,source=str(REPO if engine=='renderlab' else ROOT/'ara-main'),
            kind='Rust adapter edit',scope='optimized Cargo rebuild of staged measurement adapter; renderer source unchanged; output exposure +0.125 stops',
            commands=[build,command],ready=str(capture/'first-image.ready'),out=str(out/'result'),staged_edit=str(project/'main.rs'),
            edit_find=find,edit_replace=replace,environment={'CARGO_TARGET_DIR':str(target),'CARGO_BUILD_JOBS':'1','RENDERLAB_EXPERIMENTAL_RAY_QUERY_ACK':'I_ACCEPT_EXPERIMENTAL_WGPU_UB_RISK'}))
        save(project/'prebuild.json',dict(command=build,target=str(target)))
    print(stage)


if __name__=='__main__':main()
