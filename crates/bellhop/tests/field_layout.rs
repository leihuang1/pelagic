use bellhop::model::ReceiverGrid;
use bellhop::result::{FieldSample, SimulationResult, SourceField};
use num_complex::Complex32;

#[test]
#[allow(clippy::cast_precision_loss)]
fn native_and_language_arrays_share_depth_major_order_without_inventing_receivers() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/golden/Field_G.env");
    let mut definition = bellhop::legacy::load_case(&path)
        .unwrap()
        .value
        .into_definition();
    definition.environment.positions.source_depths_m = vec![25.0];
    definition.environment.positions.receiver_depths_m = vec![20.0, 30.0];
    definition.environment.positions.receiver_ranges_m = vec![100.0, 200.0, 300.0];
    let case = bellhop::Case::from_definition(definition.clone())
        .unwrap()
        .value;
    let mut result = SimulationResult {
        title: String::new(),
        frequency_hz: 1.0,
        legacy_run_options: String::new(),
        sources: vec![],
        arrival_sources: vec![],
        eigenray_sources: vec![],
        field_sources: vec![SourceField {
            source_depth_m: 25.0,
            samples: (0..6)
                .map(|i| FieldSample {
                    range_m: 0.0,
                    depth_m: 0.0,
                    pressure: Complex32::new(i as f32, 0.0),
                })
                .collect(),
        }],
    };
    let grid = result.field_grid(&case).unwrap();
    assert_eq!(grid.shape(), vec![1, 2, 3]);
    assert_eq!(
        grid.pressures().map(|p| p.re).collect::<Vec<_>>(),
        vec![0., 2., 4., 1., 3., 5.]
    );
    definition.environment.run.receiver_grid = ReceiverGrid::Irregular;
    definition
        .environment
        .positions
        .receiver_depths_m
        .push(40.0);
    let case = bellhop::Case::from_definition(definition).unwrap().value;
    result.field_sources[0].samples.truncate(3);
    let grid = result.field_grid(&case).unwrap();
    assert_eq!(grid.shape(), vec![1, 3]);
    assert_eq!(
        grid.pressures().map(|p| p.re).collect::<Vec<_>>(),
        vec![0., 1., 2.]
    );
    result.field_sources[0].samples.pop();
    assert!(result.field_grid(&case).is_err());
}
