        visible+=select(1u,0u,blocked);
        if probe_witness && blocked {atomicOr(&probe_masks[probe_seed*8u+j/32u],1u<<(j%32u));}
        if probe_witness && probe_seed==0u {
            let base=j*12u;
            probe_rays[base]=vec4<f32>(ray_origin,receiver.epsilon);
            // The endpoint expression is identical to the production ray's.
            var end=ray_projected.z/rate;
            for(var axis=0u;axis<2u;axis+=1u) {
                if delta[axis]>0.0 {end=min(end,(1.0-ray_projected[axis])/delta[axis]);}
                if delta[axis]<0.0 {end=min(end,(-1.0-ray_projected[axis])/delta[axis]);}
            }
            probe_rays[base+1u]=vec4<f32>(direction,end);
            probe_rays[base+11u]=vec4<f32>(receiver.geometric,receiver.epsilon);
            if blocked {
                // Replay the same descriptor with vertex return; the production
                // estimator's committed hit remains its visibility decision.
                rayQueryInitialize(&vertices,probe_scene,RayDesc(RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_TERMINATE_ON_FIRST_HIT,255u,receiver.epsilon,end,ray_origin,direction));
                while rayQueryProceed(&vertices) {}
                let hit=rayQueryGetCommittedIntersection(&vertices);
                probe_rays[base+2u]=vec4<f32>(hit.t,hit.barycentrics,bitcast<f32>(hit.kind));
                probe_rays[base+3u]=bitcast<vec4<f32>>(vec4<u32>(hit.instance_custom_data,hit.primitive_index,hit.instance_index,0u));
                let h=rayQueryGetCommittedIntersection(&vertices);
                let v=getCommittedHitVertexPositions(&vertices);
                for(var k=0u;k<3u;k+=1u) {probe_rays[base+4u+k]=vec4<f32>(v[k],1.0);}
                for(var k=0u;k<4u;k+=1u) {probe_rays[base+7u+k]=vec4<f32>(h.object_to_world[k],0.0);}
            }
        }
