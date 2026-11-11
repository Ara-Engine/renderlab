"""N-VSM2: a measurement variant of shaders/virtual_shadow_disk.wgsl for RENDERLAB_VD_SHADER.

    py -3 -B tools/n-vsm1/variant.py OUT.wgsl [SOURCE.wgsl]

Adds `override VD_DEBUG` (set through RENDERLAB_VD) so the lookup writes a diagnostic in place of
the visible count (the benches read it back as count / 256):
  1  the receiver's level and its map cell: level * 32 + page parity (16 x, 8 y) + a 4-texel
     checker (4) + texel parity (2 x, 1 y); 255 where no level holds the receiver.
  2  top-layer texels read by the map windows, divided by 3.
  3  max-mip cells read by the level windows (N-VSM2 step 2) and by the bands above the
     coarsest level, divided by 3.
  4  the receiver's level plus one, times 16, where the map windows blocked anything; 0 elsewhere.
The shipped shader carries none of this (dead debug code has cost the lookups before).
"""
from pathlib import Path
import sys

REPO = Path(__file__).resolve().parents[2]


def patch(source, old, new, count=1):
    assert source.count(old) == count, (old, source.count(old))
    return source.replace(old, new)


def debug(s):
    s = patch(s, "override VD_SKIP:u32=0u;", "override VD_SKIP:u32=0u;\noverride VD_DEBUG:u32=0u;\nvar<private> vd_texels:u32;\nvar<private> vd_cells:u32;")
    s = patch(s, "    let top=vsm_top_words[i];\n", "    let top=vsm_top_words[i];\n    vd_texels+=1u;\n")
    # The level windows' first max-mip cells (N-VSM2 step 2) count with the bands' cells.
    cell = "                    let top=fma(vsm_mips[first+u32(cy*cells+cx)],vsm.depth.z,vsm.depth.x)-p.z;\n"
    if cell in s:
        s = patch(s, cell, cell + "                    vd_cells+=1u;\n")
    s = patch(s, "                let height=fma(vsm_mips[base+u32(y*s+cx)],vsm.depth.z,vsm.depth.x);\n",
              "                let height=fma(vsm_mips[base+u32(y*s+cx)],vsm.depth.z,vsm.depth.x);\n                vd_cells+=1u;\n")
    s = patch(s, "    vd_mask=vec2<u32>(0u);\n", """    vd_mask=vec2<u32>(0u);
    vd_texels=0u;vd_cells=0u;
    if VD_DEBUG==1u {
        if vd_px.level==VSM_NONE {return 255u;}
        let c=vec2<u32>(floor((vd_px.p.xy-vsm.levels[vd_px.level].origin.xy)/vsm_texel(vd_px.level)));
        let page=c/VSM_PAGE;
        return vd_px.level*32u+(page.x&1u)*16u+(page.y&1u)*8u+(((c.x>>2u)^(c.y>>2u))&1u)*4u+(c.x&1u)*2u+(c.y&1u);
    }
""")
    s = patch(s, "    if vd_px.level!=VSM_NONE && VD_SKIP!=3u {vd_map();}\n", """    if vd_px.level!=VSM_NONE && VD_SKIP!=3u {vd_map();}
    if VD_DEBUG==2u {return min(vd_texels/3u,256u);}
    if VD_DEBUG==3u {return min(vd_cells/3u,256u);}
    if VD_DEBUG==4u {return select(0u,(vd_px.level+1u)*16u,any(vd_mask!=vec2<u32>(0u)) && vd_px.level!=VSM_NONE);}
""")
    return s


if __name__ == "__main__":
    source = Path(sys.argv[2] if len(sys.argv) > 2 else REPO / "shaders/virtual_shadow_disk.wgsl").read_bytes().decode("utf-8")
    Path(sys.argv[1]).write_bytes(debug(source).encode("utf-8"))
