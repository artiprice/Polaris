use polaris_tools_math::{
    ComputeCapability, Finite, GpuSpecTable, Interp, Knot, LaunchConfig, Limiter, Point2, bezier,
    bilinear, effective_bandwidth, interpolate, occupancy, peak_bandwidth, summarize,
};
use rust_decimal::Decimal;

fn f(value: f64) -> Finite {
    Finite::new(value).unwrap_or(Finite::ZERO)
}

#[test]
fn linear_hits_knots_and_rejects_outside_domain() {
    let knots = [
        Knot {
            x: f(0.0),
            y: f(2.0),
        },
        Knot {
            x: f(1.0),
            y: f(5.0),
        },
    ];
    assert_eq!(
        interpolate(&knots, Interp::Linear, f(1.0)).map(Finite::get),
        Ok(5.0)
    );
    assert!(interpolate(&knots, Interp::Linear, f(2.0)).is_err());
}

#[test]
fn catmull_rom_returns_knots_and_rejects_nonuniform_spacing() {
    let knots = [0.0, 1.0, 2.0, 3.0].map(|x| Knot {
        x: f(x),
        y: f(x * x),
    });
    assert_eq!(
        interpolate(&knots, Interp::CatmullRom, f(2.0)).map(Finite::get),
        Ok(4.0)
    );
    let uneven = [0.0, 1.0, 2.2, 3.0].map(|x| Knot { x: f(x), y: f(x) });
    assert!(interpolate(&uneven, Interp::CatmullRom, f(1.5)).is_err());
}

#[test]
fn bilinear_reproduces_corners_and_bezier_stays_bounded() {
    let grid = polaris_tools_math::Grid2 {
        x: [f(0.0), f(1.0)],
        y: [f(0.0), f(1.0)],
        z: [f(0.0), f(1.0), f(2.0), f(3.0)],
    };
    assert_eq!(bilinear(&grid, f(1.0), f(1.0)).map(Finite::get), Ok(3.0));
    let point = bezier(
        &[
            Point2 {
                x: f(0.0),
                y: f(0.0),
            },
            Point2 {
                x: f(1.0),
                y: f(2.0),
            },
            Point2 {
                x: f(2.0),
                y: f(0.0),
            },
        ],
        f(0.5),
    );
    assert!(point.is_ok_and(|p| (0.99..=1.01).contains(&p.x.get()) && (0.99..=1.01).contains(&p.y.get())));
}

#[test]
fn summary_is_finite_ordered_and_singleton_stddev_zero() {
    assert!(summarize(&[]).is_err());
    let values = [f(1.0), f(2.0), f(3.0), f(4.0)];
    let summary = summarize(&values);
    assert!(summary.is_ok_and(|s| s.mean.get() == 2.5
        && s.std_dev.get() > 0.0
        && s.min.get() <= s.p50.get()
        && s.p50.get() <= s.p95.get()
        && s.p95.get() <= s.p99.get()
        && s.p99.get() <= s.max.get()));
    assert_eq!(summarize(&[f(7.0)]).map(|s| s.std_dev.get()), Ok(0.0));
}

#[test]
fn gpu_occupancy_identifies_resource_and_rejects_unknown_capability() {
    let table_src = include_str!("../src/gpu_specs.toml");
    let table = GpuSpecTable::from_toml(table_src);
    assert!(table.is_ok());
    if let Ok(table) = table {
        let launch = LaunchConfig {
            threads_per_block: 128,
            regs_per_thread: 128,
            shared_bytes_per_block: 0,
        };
        let result = occupancy(&table, ComputeCapability { major: 8, minor: 9 }, &launch);
        assert!(result.is_ok_and(|o| o.blocks_per_sm == 4
            && o.limiter == Limiter::Registers
            && o.fraction_milli <= 1000));
        let shared_limited = LaunchConfig {
            shared_bytes_per_block: 65_536,
            ..launch
        };
        assert!(
            occupancy(
                &table,
                ComputeCapability { major: 8, minor: 9 },
                &shared_limited
            )
            .is_ok_and(|o| o.blocks_per_sm == 1 && o.limiter == Limiter::SharedMemory)
        );
        assert!(occupancy(&table, ComputeCapability { major: 9, minor: 2 }, &launch).is_err());
    }
}

#[test]
fn gpu_rows_require_valid_source_and_limits() {
    let table = include_str!("../src/gpu_specs.toml").replace("https://", "http://");
    assert!(GpuSpecTable::from_toml(&table).is_err());
    let missing = include_str!("../src/gpu_specs.toml").replace("source_url = \"https://docs.nvidia.com/cuda/cuda-programming-guide/05-appendices/compute-capabilities.html\"\n", "");
    assert!(GpuSpecTable::from_toml(&missing).is_err());
    let invalid_date = include_str!("../src/gpu_specs.toml").replace("2026-10-23", "2026-02-31");
    assert!(GpuSpecTable::from_toml(&invalid_date).is_err());
}

#[test]
fn curated_nvidia_reference_has_sourced_rows_for_supported_capabilities() {
    let table = GpuSpecTable::nvidia_reference();
    assert!(table.is_ok());
    if let Ok(table) = table {
        for capability in [
            ComputeCapability { major: 7, minor: 5 },
            ComputeCapability { major: 8, minor: 0 },
            ComputeCapability { major: 8, minor: 6 },
            ComputeCapability { major: 8, minor: 7 },
            ComputeCapability { major: 8, minor: 9 },
            ComputeCapability { major: 9, minor: 0 },
        ] {
            assert!(
                table
                    .spec(capability)
                    .is_some_and(|row| row.source_url.starts_with("https://docs.nvidia.com/")
                        && row.retrieved == "2026-10-23")
            );
        }
    }
}

#[test]
fn bandwidth_is_decimal_gigabytes_per_second() {
    assert_eq!(
        effective_bandwidth(1_000_000_000, Decimal::ONE).map(|r| r.value()),
        Ok(Decimal::ONE)
    );
    assert!(effective_bandwidth(1, Decimal::ZERO).is_err());
    assert_eq!(
        peak_bandwidth(Decimal::from(16_u32), 256).map(|r| r.value()),
        Ok(Decimal::from(512_u32))
    );
}
