//! Exact barrier contracts, independent of Metal's conservative stage synchronization.
use super::*;
use ara_gpu::{
    Image, ImageKind,
    arena::Arena,
    sync::{BarrierRange, ResourceBarrier},
    texture::{ImageViewDesc, ViewDimension},
};
use renderlab::backend_ara::graph::{ResourceUse, Subresources};
fn usage(image: Image, mip: u32, layer: u32, access: Access) -> ResourceUse {
    ResourceUse::Image {
        image,
        allocation: Subresources { mips: 3, layers: 2 },
        view: ImageViewDesc {
            dimension: ViewDimension::D2Array,
            base_mip: mip,
            mip_count: 1,
            base_layer: layer,
            layer_count: 1,
        },
        access,
    }
}
fn cells(image: Image, barriers: &[ResourceBarrier]) -> Vec<(u32, u32, Access, Access)> {
    let mut out = Vec::new();
    for b in barriers {
        assert_eq!(b.barrier.resource, Resource::Image(image));
        let (m, c, l, n) = match b.range {
            BarrierRange::Whole => (0, 3, 0, 2),
            BarrierRange::Image {
                base_mip,
                mip_count,
                base_layer,
                layer_count,
            } => (base_mip, mip_count, base_layer, layer_count),
            _ => panic!("image received buffer range"),
        };
        for mip in m..m + c {
            for layer in l..l + n {
                out.push((mip, layer, b.barrier.before, b.barrier.after));
            }
        }
    }
    out.sort_by_key(|(m, l, _, _)| (*m, *l));
    out
}
#[test]
fn image_barrier_plan_ranges_states_and_transactional_rejection() -> Result<()> {
    let mut arena = Arena::<ImageKind, ()>::default();
    let image = arena.insert(())?;
    let mut graph = Graph::default();
    graph.import(Resource::Image(image), Access::CopyDestination);
    let b = graph.plan(&[
        usage(image, 0, 0, Access::ShaderRead),
        usage(image, 0, 1, Access::ShaderRead),
    ])?;
    assert_eq!(
        cells(image, &b),
        vec![
            (0, 0, Access::CopyDestination, Access::ShaderRead),
            (0, 1, Access::CopyDestination, Access::ShaderRead)
        ]
    );
    let b = graph.plan(&[
        usage(image, 0, 0, Access::ShaderRead),
        usage(image, 1, 0, Access::Storage),
    ])?;
    assert_eq!(
        cells(image, &b),
        vec![(1, 0, Access::CopyDestination, Access::Storage)]
    );
    let b = graph.plan(&[ResourceUse::Whole(
        Resource::Image(image),
        Access::CopySource,
    )])?;
    let expected: Vec<_> = (0..3)
        .flat_map(|m| {
            (0..2).map(move |l| {
                (
                    m,
                    l,
                    if m == 0 {
                        Access::ShaderRead
                    } else if m == 1 && l == 0 {
                        Access::Storage
                    } else {
                        Access::CopyDestination
                    },
                    Access::CopySource,
                )
            })
        })
        .collect();
    assert_eq!(cells(image, &b), expected);
    graph.import(Resource::Image(image), Access::HostWrite);
    let b = graph.plan(&[ResourceUse::Whole(
        Resource::Image(image),
        Access::ShaderRead,
    )])?;
    assert_eq!(
        cells(image, &b),
        (0..3)
            .flat_map(|m| (0..2).map(move |l| (m, l, Access::HostWrite, Access::ShaderRead)))
            .collect::<Vec<_>>()
    );
    assert!(
        graph
            .plan(&[
                usage(image, 0, 0, Access::Storage),
                usage(image, 0, 0, Access::ShaderRead)
            ])
            .is_err()
    );
    assert!(
        graph
            .plan(&[usage(image, 0, 0, Access::ShaderRead)])?
            .is_empty()
    );
    let b = graph.plan(&[
        usage(image, 0, 0, Access::ShaderRead),
        usage(image, 0, 1, Access::Storage),
    ])?;
    assert_eq!(
        cells(image, &b),
        vec![(0, 1, Access::ShaderRead, Access::Storage)]
    );
    graph.forget(Resource::Image(image));
    assert!(
        graph
            .plan(&[usage(image, 0, 0, Access::ShaderRead)])
            .is_err()
    );
    // A failed read must not register bounds. Fresh successful declarations can
    // supply their actual allocation without inheriting failed metadata/states.
    let one = ResourceUse::Image {
        image,
        allocation: Subresources { mips: 1, layers: 1 },
        view: ImageViewDesc {
            dimension: ViewDimension::D2,
            base_mip: 0,
            mip_count: 1,
            base_layer: 0,
            layer_count: 1,
        },
        access: Access::Storage,
    };
    let b = graph.plan(&[one])?;
    assert_eq!(
        cells(image, &b),
        vec![(0, 0, Access::Undefined, Access::Storage)]
    );
    let other = arena.insert(())?;
    graph.import(Resource::Image(other), Access::CopyDestination);
    assert!(
        graph
            .plan(&[
                ResourceUse::Whole(Resource::Image(other), Access::Storage),
                usage(other, 2, 1, Access::ShaderRead)
            ])
            .is_err()
    );
    assert_eq!(
        cells(
            other,
            &graph.plan(&[usage(other, 2, 1, Access::ShaderRead)])?
        ),
        vec![(2, 1, Access::CopyDestination, Access::ShaderRead)]
    );
    let shared = usage(other, 2, 1, Access::Storage);
    assert_eq!(
        cells(other, &graph.plan(&[shared, shared])?),
        vec![(2, 1, Access::ShaderRead, Access::Storage)]
    );
    assert_eq!(
        cells(other, &graph.plan(&[shared])?),
        vec![(2, 1, Access::Storage, Access::Storage)]
    );
    Ok(())
}

#[test]
fn shader_and_indirect_reads_share_one_buffer_state() -> Result<()> {
    let mut arena = Arena::<ara_gpu::BufferKind, ()>::default();
    let resource = Resource::Buffer(arena.insert(())?);
    for accesses in [
        [Access::ShaderRead, Access::Indirect],
        [Access::Indirect, Access::ShaderRead],
    ] {
        let mut graph = Graph::default();
        graph.import(resource, Access::CopyDestination);
        let barriers = graph.plan(&accesses.map(|a| ResourceUse::Whole(resource, a)))?;
        assert_eq!(barriers.len(), 1);
        assert_eq!(barriers[0].barrier.before, Access::CopyDestination);
        assert_eq!(barriers[0].barrier.after, Access::Indirect);
        let b = graph.plan(&[ResourceUse::Whole(resource, Access::Storage)])?;
        assert_eq!(b[0].barrier.before, Access::Indirect);
        assert!(
            graph
                .plan(&[
                    ResourceUse::Whole(resource, Access::Storage),
                    ResourceUse::Whole(resource, Access::ShaderRead)
                ])
                .is_err()
        );
        let b = graph.plan(&[ResourceUse::Whole(resource, Access::ShaderRead)])?;
        assert_eq!(b[0].barrier.before, Access::Storage);
    }
    Ok(())
}
