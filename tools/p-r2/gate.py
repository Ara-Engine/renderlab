"""CPU replay of R2's scheduling gate from diagnostic count captures.

The diagnostic program writes exposed HDR (count / 16, 1, 0), where count is
the RQ1 loop bound after R1's lobe/view rejection. RF captures are raw RGB f32
radiance; ARA1 captures are PNGs through the Filmic display transform.
This measures loop-bound divergence, not BVH traversal or hit-shading variance.
"""
import argparse
import json
from pathlib import Path

import numpy as np
from PIL import Image


def diagnostic_program(standard):
    """Keep production surface/lobe evaluation; replace only the final output."""
    return standard.split('fn shade(')[0] + '''// Retain the API types used by standard's lobe helpers, without a cache read.
fn pr2_unused_input(input:SurfaceInput)->vec3<f32> {
 return indirect_radiance(input,vec3<f32>(0.0,1.0,0.0),0.0);
}
fn shade(s:Surface,input:SurfaceInput)->vec3<f32> {
 let lobe=reflection_lobe(s,input);
 let rough=sqrt(clamp(lobe.alpha,0.0,1.0));
 let n=unit_normal(lobe.normal);let v=unit_normal(input.eye-input.world);
 var count=0u;
 if rough<0.5 && lobe.weight>0.0 && dot(n,n)>=0.5 && dot(v,v)>=0.5 && dot(n,v)>1e-4 {
  count=select(clamp(u32(ceil(48.0*rough)),4u,16u),1u,rough<0.05);
 }
 // Exposed HDR integer code; green and blue mark geometry versus background.
 return vec3<f32>(f32(count)/16.0,1.0,0.0);
}
'''


def balance(counts):
    """Replay x-fastest 8x4 shading groups, padding image edges with zero work."""
    height, width = counts.shape
    padded = np.pad(counts, ((0, -height % 4), (0, -width % 8)))
    tiles = padded.reshape(padded.shape[0] // 4, 4, padded.shape[1] // 8, 8)
    totals = tiles.sum(axis=(1, 3))
    maxima = tiles.max(axis=(1, 3))
    rays = int(totals.sum())
    return dict(width=width, height=height, rays=rays,
                active_tiles=int(np.count_nonzero(totals)),
                sum_max=int(maxima.sum()), sum_mean=rays / 32,
                ratio=float(maxima.sum() * 32 / rays) if rays else None,
                histogram=np.bincount(counts.ravel(), minlength=17).tolist())


def png_counts(path):
    rgb = np.array(Image.open(path))[:, :, :3].astype(np.float64)
    x = np.arange(17) / 16
    tone = np.clip(x * (2.51*x + .03) / (x * (2.43*x + .59) + .14), 0, 1)
    srgb = np.where(tone <= .0031308, 12.92*tone, 1.055*tone**(1/2.4)-.055)
    codes = np.round(srgb * 255)
    assert len(np.unique(codes)) == 17
    # Green marks diagnostic geometry, blue excludes the lit background.
    geometry = (np.abs(rgb[:, :, 1] - codes[16]) <= 1) & (rgb[:, :, 2] == 0)
    if not np.any(geometry):
        raise ValueError(f"{path}: no diagnostic geometry found")
    error = np.abs(rgb[:, :, 0, None] - codes)
    if np.any(error.min(axis=2)[geometry] > 1):
        raise ValueError(f"{path}: count encoding changed by display/reconstruction")
    return np.where(geometry, error.argmin(axis=2), 0).astype(np.uint8)


def hdr_counts(path):
    metadata = json.loads(path.with_suffix('.json').read_text())
    rgb = np.fromfile(path, dtype='<f4').reshape(metadata['height'], metadata['width'], 3)
    marker = 2 ** -metadata['exposure_stops']
    geometry = (rgb[:, :, 1] == marker) & (rgb[:, :, 2] == 0)
    if not np.any(geometry):
        raise ValueError(f"{path}: no diagnostic geometry found")
    count = rgb[:, :, 0] / marker * 16
    if np.any((count[geometry] < 0) | (count[geometry] > 16)):
        raise ValueError(f"{path}: count outside RQ1's 0..16 range")
    if np.any(np.abs(count[geometry] - np.round(count[geometry])) > .001):
        raise ValueError(f"{path}: noninteger count encoding")
    return np.where(geometry, np.round(count), 0).astype(np.uint8)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('root', type=Path)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--program', action='store_true',
                        help='write the diagnostic WGSL instead of analysing captures')
    args = parser.parse_args()
    if args.program:
        args.out.write_text(diagnostic_program(args.root.read_text()))
        return
    rows = {}
    for path in sorted(args.root.rglob('*.png')):
        rows[str(path.relative_to(args.root))] = balance(png_counts(path))
    for path in sorted(args.root.rglob('*.f32')):
        rows[str(path.relative_to(args.root))] = balance(hdr_counts(path))
    args.out.write_text(json.dumps(rows, indent=2) + '\n')
    for name, row in rows.items():
        print(name, row['ratio'], 'rays', row['rays'])


if __name__ == '__main__':
    main()
