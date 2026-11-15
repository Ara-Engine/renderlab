        let local_vertex=(t>>(8u*(vertex%3u)))&255u;
        if REUSE_VERTICES {
            let at=c.pad1+local_vertex;
            out.position=textureLoad(projected,vec2<i32>(i32(at&1023u),i32(at>>10u)),0);
            return out;
        }
        let o=c.vertices+3u*local_vertex;let d=&cluster_data[batch].data;
