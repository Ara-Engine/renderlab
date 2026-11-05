"""Focused CPU check: surface rejection conditions visibility, never radiance."""
import importlib.util
from pathlib import Path
import tempfile
import numpy as np

spec=importlib.util.spec_from_file_location("r1_score",Path(__file__).with_name("m6-receiver-score.py"))
score=importlib.util.module_from_spec(spec);spec.loader.exec_module(score)

def main():
    root=Path("D:/Coding/renderlab-work/m6-exact-quality/r1")
    with tempfile.TemporaryDirectory(prefix="replay-check-",dir=root) as temporary:
        directory=Path(temporary)
        assert directory.resolve().is_relative_to(root.resolve())
        (directory/"samples").mkdir()
        frames=[]
        for i in range(16):
            rgb=np.array([100. if i==0 else i+1.,i+1.,2*(i+1.)])
            frame=np.ones((1,3,4),dtype="<f2");frame[0,:,:3]=rgb[:,None];frames.append(frame.astype(float))
            frame.tofile(directory/f"samples/{i:02}.rgba16f")
            sun=frame.copy();sun[...,:3]*=2;sun.tofile(directory/f"samples/{i:02}.sun.rgba16f")
            np.array([[65535 if i==0 else 160,160,65535]],dtype="<u2").tofile(directory/f"samples/{i:02}.visibility.u16")
        mean=np.mean(frames,axis=0)
        cover=np.array([[True,True,False]])
        reference=dict(ref=np.array([[.5,.5,1.]]),hdr=mean,cover=cover)
        direct=mean.copy();direct[...,:3]*=2
        exact=mean.copy();exact[...,:3]+=1
        exact_sun=direct.copy();exact_sun[...,:3]+=1
        result=score.replay(directory,0,reference,direct,np.ones((1,3),bool),np.array([[.75,.75,1.]]),exact,exact_sun)
        expected=float(np.sqrt(np.mean((frames[0][...,:3]-mean[...,:3])**2)))
        assert np.isclose(result["curve"][0]["radiance_rms"],expected)
        assert np.isclose(result["curve"][0]["sun_rms"],2*expected)
        assert result["curve"][0]["missing"]==1
        assert result["fits"]["visibility_rms"]["exact_point_rms"]==.25
        print("PASS: rejected foreground and background samples stay in radiance; visibility baseline/missing counts use covered receivers")

if __name__=="__main__":main()
