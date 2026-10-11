use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use bellhop::legacy::load_case;
use bellhop::solver::{RayTermination, SimulationLimits, run};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "bellhop-ray-test-{}-{sequence}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn traces_official_munk_ray_fan_deterministically() {
    let case = load_case(&fixture("MunkB_ray.env")).unwrap().value;
    let first = run(&case, SimulationLimits::default()).unwrap();
    let second = run(&case, SimulationLimits::default()).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.sources.len(), 2);
    assert_eq!(first.sources[0].rays.len(), 41);
    assert!(
        first
            .sources
            .iter()
            .flat_map(|source| &source.rays)
            .all(|ray| !ray.points.is_empty())
    );
    assert!(
        first
            .sources
            .iter()
            .flat_map(|source| &source.rays)
            .all(|ray| ray.termination != RayTermination::StepLimit)
    );
}

#[test]
fn legacy_rg_json_round_trip_preserves_ray_outcomes() {
    use bellhop::json::{export_case_document, load_case_document};

    // Six SSP models, material/IRC reflection, shaped boundaries and a multi-source fan.
    // Derived RG inputs: the committed reference fixtures remain unchanged.
    for name in [
        "golden/N2_one_ray.env",
        "golden/CLinear_one_ray.env",
        "golden/Spline_one_ray.env",
        "golden/MunkP_one_ray.env",
        "golden/Quadrilateral_one_ray.env",
        "golden/Analytic_one_ray.env",
        "golden/ElasticReflection.env",
        "golden/GrainReflection.env",
        "golden/InternalReflection.env",
        "golden/DickinsCritical.env",
        "golden/ParaBotCritical.env",
        "MunkB_ray.env",
    ] {
        let directory = TemporaryDirectory::new();
        let path = directory.join("case.env");
        let (_, source, inputs) = bellhop::legacy::load_case_with_inputs(&fixture(name))
            .unwrap()
            .into_parts();
        for input in inputs.iter().skip(1) {
            fs::copy(input, path.with_extension(input.extension().unwrap())).unwrap();
        }
        for options in ["RG", "RG    S"] {
            let derived = source.replace("'R'", &format!("'{options}'"));
            assert_ne!(derived, source, "{name}: expected a ray run record");
            fs::write(&path, derived).unwrap();
            let legacy = load_case(&path).unwrap().value;
            assert!(legacy.environment.run.legacy.starts_with("RG"));
            assert!(legacy.environment.run.beam_family.is_none());
            let document = export_case_document(&legacy).unwrap();
            let json = serde_json::to_vec(&document).unwrap();
            let modern = load_case_document(&json).unwrap().value;
            assert_eq!(modern.environment.run.legacy, "");
            assert!(modern.environment.run.beam_family.is_none());
            for max_steps_per_ray in [SimulationLimits::default().max_steps_per_ray, 2] {
                let limits = SimulationLimits {
                    max_steps_per_ray,
                    ..SimulationLimits::default()
                };
                let expected = run(&legacy, limits);
                let actual = run(&modern, limits);
                if let Err(expected) = expected {
                    // This derived beam-shift fan reaches a non-finite reflection on
                    // the host. JSON must retain the failure, not turn it into success.
                    assert_eq!((name, options), ("MunkB_ray.env", "RG    S"));
                    assert!(max_steps_per_ray > 2);
                    let actual = actual.unwrap_err();
                    assert_eq!(expected.diagnostics().len(), 1);
                    assert_eq!(actual.diagnostics().len(), 1);
                    let mut expected = expected.diagnostics()[0].clone();
                    assert_eq!(expected.code, "BH0302");
                    assert!(expected.message.contains("non-finite ray state"));
                    expected
                        .location
                        .path
                        .clone_from(&actual.diagnostics()[0].location.path);
                    assert_eq!(expected, actual.diagnostics()[0]);
                    continue;
                }
                let mut expected = expected.unwrap();
                let actual = actual.unwrap();
                // Provenance is intentionally different; compare every exported numerical
                // product, ray count, bounce count and termination, not option strings.
                assert_ne!(expected.legacy_run_options, actual.legacy_run_options);
                expected
                    .legacy_run_options
                    .clone_from(&actual.legacy_run_options);
                assert_eq!(
                    expected, actual,
                    "{name}: {options}, step limit {max_steps_per_ray}"
                );
                for (a, b) in expected.sources.iter().zip(&actual.sources) {
                    assert_eq!(a.source_depth_m.to_bits(), b.source_depth_m.to_bits());
                    for (a, b) in a.rays.iter().zip(&b.rays) {
                        assert_eq!(
                            a.launch_angle_degrees.to_bits(),
                            b.launch_angle_degrees.to_bits()
                        );
                        for (a, b) in a.points.iter().zip(&b.points) {
                            let bits = |p: &bellhop::result::RayPoint| {
                                [
                                    p.range_m,
                                    p.depth_m,
                                    p.travel_time_s,
                                    p.attenuation_time_s,
                                    p.amplitude,
                                    p.phase_radians,
                                ]
                                .map(f64::to_bits)
                            };
                            assert_eq!(bits(a), bits(b), "{name}: {options}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn rejects_ray_fans_above_the_configured_limit() {
    let case = load_case(&fixture("MunkB_ray.env")).unwrap().value;
    let report = run(
        &case,
        SimulationLimits {
            max_rays: 1,
            ..SimulationLimits::default()
        },
    )
    .unwrap_err();
    assert_eq!(report.diagnostics()[0].code, "BH0303");
}

#[test]
fn applies_tabulated_and_material_boundary_reflections() {
    for (name, bottom_options, bottom_data) in [
        ("table", "'F' 0.0", ""),
        ("elastic", "'A' 0.0", "100.0 1800.0 600.0 2.0 0.1 0.2 /\n"),
        ("grain", "'G' 0.0", "100.0 1.5\n"),
    ] {
        let directory = TemporaryDirectory::new();
        let environment = directory.join(&format!("{name}.env"));
        fs::write(
            &environment,
            format!(
                "'Boundary reflection'\n\
                 100.0\n\
                 1\n\
                 'CRW'\n\
                 2 0.0 100.0\n\
                 0.0 1500.0 /\n\
                 100.0 1500.0 /\n\
                 {bottom_options}\n\
                 {bottom_data}\
                 1\n\
                 50.0 /\n\
                 2\n\
                 0.0 100.0 /\n\
                 2\n\
                 0.0 1.0 /\n\
                 'R'\n\
                 1\n\
                 30.0 /\n\
                 1.0 101.0 1.0\n"
            ),
        )
        .unwrap();
        if name == "table" {
            fs::write(
                directory.join("table.brc"),
                "2\n0.0 0.001 0.0\n90.0 0.001 0.0\n",
            )
            .unwrap();
        }

        let case = load_case(&environment).unwrap().value;
        let result = run(&case, SimulationLimits::default()).unwrap();
        let ray = &result.sources[0].rays[0];
        assert!(ray.bottom_bounces >= 1, "{name}");
        if name == "table" {
            assert_eq!(ray.termination, RayTermination::LostEnergy);
        }
    }
}
