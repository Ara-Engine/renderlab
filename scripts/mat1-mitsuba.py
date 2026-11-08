import json,math,argparse
from pathlib import Path
import mitsuba as mi
import numpy as np
mi.set_variant('scalar_rgb')
p=argparse.ArgumentParser();p.add_argument('output',type=Path);args=p.parse_args()
# Metallic Schlick with base .04 isolates the same Fresnel law as Renderlab's dielectric lobe.
bsdf=mi.load_dict({'type':'principled','base_color':{'type':'rgb','value':[.04]*3},'metallic':1.,'roughness':.85})
context=mi.BSDFContext();context.type_mask=mi.BSDFFlags.GlossyReflection
rows=[]
for nv in [.15,.3,.5,.8,1.]:
 v=np.array([math.sqrt(1-nv*nv),0,nv]);interaction=mi.SurfaceInteraction3f();interaction.wi=mi.Vector3f(v)
 for nl in [.1,.3,.7,1.]:
  for phi in [0.,1.,2.,3.]:
   light=np.array([math.sqrt(1-nl*nl)*math.cos(phi),math.sqrt(1-nl*nl)*math.sin(phi),nl]);half=v+light;half/=np.linalg.norm(half)
   a2=.85**4;distribution=a2/(math.pi*(1+(a2-1)*half[2]**2)**2)
   g1=lambda cosine:2*cosine/(cosine+math.sqrt(a2+(1-a2)*cosine*cosine))
   fresnel=.04+.96*(1-np.dot(v,half))**5
   expected=distribution*g1(nv)*g1(nl)/(4*nv)*fresnel
   actual=float(np.array(bsdf.eval(context,interaction,mi.Vector3f(light)))[0])
   rows.append(dict(nv=nv,nl=nl,phi=phi,mitsuba=actual,model=expected,error=actual-expected))
report=dict(mitsuba=mi.__version__,smith='separable',fresnel='Schlick F0 .04',roughness=.85,
 max_abs_error=max(abs(x['error']) for x in rows),max_relative_error=max(abs(x['error'])/max(x['model'],1e-20) for x in rows),rows=rows)
assert report['max_relative_error']<2e-6
args.output.write_text(json.dumps(report,indent=2))
print({k:v for k,v in report.items() if k!='rows'})
