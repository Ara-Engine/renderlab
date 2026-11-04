use renderlab_cook::environment::*;

#[test]
fn full_field_work_is_explicit_and_caller_policy_wins() {
    let input =
        LinearEnvironment::new(1024, 512, vec![[0.0; 3]; 1024 * 512], CookLimits::default())
            .unwrap();
    assert!(estimate_field_cost(&input, CookLimits::default()).is_err());
    let limits = CookLimits {
        max_sample_evaluations: 104_000_000_000,
        ..Default::default()
    };
    let cost = estimate_field_cost(&input, limits).unwrap();
    assert_eq!(cost.sample_evaluations, 103_105_552_384);
    assert_eq!(cost.prepared_pixel_bytes, 76_695_552);
    assert!(
        estimate_field_cost(
            &input,
            CookLimits {
                max_sample_evaluations: cost.sample_evaluations - 1,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        estimate_field_cost(
            &input,
            CookLimits {
                max_working_bytes: cost.owned_working_bytes - 1,
                ..limits
            }
        )
        .is_err()
    );
    assert!(
        estimate_field_cost(
            &input,
            CookLimits {
                prepared: renderlab_asset::environment::Limits {
                    max_payload_bytes: cost.prepared_pixel_bytes - 1,
                    ..limits.prepared
                },
                ..limits
            }
        )
        .is_err()
    );
}

#[cfg(feature = "gpu-environment")]
#[test]
fn zero_gpu_work_policy_rejects_before_device_creation() {
    let input = LinearEnvironment::new(2, 1, vec![[1.; 3]; 2], CookLimits::default()).unwrap();
    assert!(
        gpu::GpuPreparer::new(
            &input,
            CookLimits {
                max_sample_evaluations: 0,
                ..Default::default()
            },
            gpu::GpuPolicy::default()
        )
        .is_err()
    );
    assert_eq!(
        format!(
            "{:x}",
            <sha2::Sha256 as sha2::Digest>::digest(gpu::KERNEL.as_bytes())
        ),
        gpu::KERNEL_SHA256
    );
}
