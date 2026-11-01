//! Full-sequence resource probe; no numerical snapshots or in-process RSS sampling.
use std::{num::NonZeroUsize, path::Path, process::ExitCode, time::Instant};

use kraken::{DiagnosticReport, FieldCase, ProfileSimulationResult};
use serde_json::{Value, json};

const USAGE: &str = "usage: resources {stream|retain} THREADS json INPUT.json\n       resources {stream|retain} THREADS env {kraken|krakenc} INPUT.env INPUT.flp";

struct Measurement<T, E, D> {
    retained: Vec<T>,
    dimensions: Vec<D>,
    first_yield_ns: Option<u128>,
    total_ns: u128,
    error: Option<E>,
    complete: bool,
}

// Iterator construction and teardown belong to the timed production sequence.
fn consume<T, E, D, I: Iterator<Item = Result<T, E>>>(
    expected: usize,
    retain: bool,
    make_iterator: impl FnOnce() -> I,
    dimensions: impl Fn(&T) -> D,
) -> Measurement<T, E, D> {
    let mut retained = Vec::with_capacity(if retain { expected } else { 0 });
    let mut sizes = Vec::with_capacity(expected);
    let mut first_yield_ns = None;
    let mut error = None;
    let start = Instant::now();
    {
        let iterator = make_iterator();
        for item in iterator {
            if first_yield_ns.is_none() {
                first_yield_ns = Some(start.elapsed().as_nanos());
            }
            match item {
                Ok(result) => {
                    let result = std::hint::black_box(result); // Keep numerical values observable to the optimizer.
                    sizes.push(dimensions(&result));
                    if retain {
                        retained.push(result);
                    } else {
                        drop(result);
                    }
                }
                Err(diagnostic) => {
                    error = Some(diagnostic);
                    break;
                }
            }
        }
    }
    let total_ns = start.elapsed().as_nanos();
    let complete = error.is_none() && expected != 0 && sizes.len() == expected;
    Measurement {
        retained,
        dimensions: sizes,
        first_yield_ns,
        total_ns,
        error,
        complete,
    }
}

struct ProfileDimensions {
    frequency_hz: f64,
    modes: usize,
    sampled_depths: usize,
    shape_values: usize,
}

struct OutputDimensions {
    profiles: Vec<ProfileDimensions>,
    pressure_dimensions: [usize; 3],
    pressure_values: usize,
}

fn output_dimensions(result: &ProfileSimulationResult) -> OutputDimensions {
    OutputDimensions {
        profiles: result
            .modes
            .iter()
            .map(|set| ProfileDimensions {
                frequency_hz: set.frequency_hz,
                modes: set.modes.len(),
                sampled_depths: set.sampled_depths_m.len(),
                shape_values: set.modes.iter().map(|mode| mode.eigenfunction.len()).sum(),
            })
            .collect(),
        pressure_dimensions: [
            result.field.source_depths_m.len(),
            result.field.receiver_depths_m.len(),
            result.field.receiver_ranges_m.len(),
        ],
        pressure_values: result.field.pressure.len(),
    }
}

fn load(args: &[String]) -> Result<Vec<FieldCase>, Box<dyn std::error::Error>> {
    match args {
        [format, input] if format == "json" => {
            let path = Path::new(input);
            Ok(kraken::json::decode_case_document(&std::fs::read(path)?[..], path)?.cases(path)?)
        }
        [format, engine, env, flp] if format == "env" => {
            let engine = match engine.as_str() {
                "kraken" => kraken::ModeSolver::Kraken,
                "krakenc" => kraken::ModeSolver::Krakenc,
                _ => return Err(USAGE.into()),
            };
            Ok(kraken::legacy::load_field_cases(env, flp, engine)?)
        }
        _ => Err(USAGE.into()),
    }
}

fn controls(args: &[String]) -> Result<(bool, NonZeroUsize), &'static str> {
    let valid_input = match args.get(2..) {
        Some([format, _]) => format == "json",
        Some([format, engine, _, _]) => {
            format == "env" && matches!(engine.as_str(), "kraken" | "krakenc")
        }
        _ => false,
    };
    if !valid_input || args.iter().any(String::is_empty) {
        return Err(USAGE);
    }
    let retain = match args[0].as_str() {
        "stream" => false,
        "retain" => true,
        _ => return Err(USAGE),
    };
    Ok((retain, args[1].parse().map_err(|_| USAGE)?))
}

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let (retain, threads) = match controls(&args) {
        Ok(controls) => controls,
        Err(usage) => {
            eprintln!("{usage}");
            return ExitCode::from(2);
        }
    };
    let cases = match load(&args[2..]) {
        Ok(cases) => cases,
        Err(error) => {
            println!(
                "{}",
                json!({"status": "load-error", "error": error.to_string(),
                    "diagnostics": error.downcast_ref::<DiagnosticReport>().map(DiagnosticReport::diagnostics)})
            );
            return ExitCode::FAILURE;
        }
    };
    let inputs: Vec<_> = cases
        .iter()
        .map(|case| {
            let first = &case.profiles()[0];
            json!({
                "frequency_hz": first.frequency_hz,
                "engine": match first.mode_solver {
                    kraken::ModeSolver::Kraken => "kraken",
                    kraken::ModeSolver::Krakenc => "krakenc",
                },
                "profile_count": case.profiles().len(),
                "profiles": case.profiles().iter().map(|profile| json!({
                    "mode_limit": profile.mode_limit,
                    "sampled_depths": profile.mode_sample_depths_m.len(),
                })).collect::<Vec<_>>(),
                "pressure_dimensions": [first.source_depths_m.len(),
                    first.receiver_depths_m.len(), first.receiver_ranges_m.len()],
            })
        })
        .collect();
    let measurement = kraken::parallelism::with_threads(threads, || {
        consume(
            cases.len(),
            retain,
            || kraken::solve_frequencies(&cases),
            output_dimensions,
        )
    });
    // Retained numerical outputs are destroyed after the timer, before formatting.
    drop(measurement.retained);
    let outputs: Vec<Value> = measurement
        .dimensions
        .iter()
        .map(|output| {
            json!({
                "profile_count": output.profiles.len(),
                "profiles": output.profiles.iter().map(|profile| json!({
                    "frequency_hz": profile.frequency_hz,
                    "modes": profile.modes,
                    "sampled_depths": profile.sampled_depths,
                    "shape_values": profile.shape_values,
                })).collect::<Vec<_>>(),
                "pressure_dimensions": output.pressure_dimensions,
                "pressure_values": output.pressure_values,
            })
        })
        .collect();
    let status = if measurement.error.is_some() {
        "solve-error"
    } else if measurement.complete {
        "complete"
    } else {
        "incomplete"
    };
    println!(
        "{}",
        json!({
            "status": status,
            "mode": args[0],
            "threads": threads.get(),
            "expected_frequencies": cases.len(),
            "emitted_successes": outputs.len(),
            "first_yield_ns": measurement.first_yield_ns,
            "total_ns": measurement.total_ns,
            "inputs": inputs,
            "outputs": outputs,
            "error": measurement.error.as_ref().map(ToString::to_string),
            "diagnostics": measurement.error.as_ref().map(DiagnosticReport::diagnostics),
        })
    );
    if measurement.complete {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Output<'a>(&'a Cell<usize>);
    impl Drop for Output<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn resources_stream_drops_between_yields_and_retain_keeps_all() {
        for retain in [false, true] {
            let dropped = Cell::new(0);
            let measurement = consume(
                3,
                retain,
                || {
                    (0..3).map(|index| {
                        assert_eq!(dropped.get(), if retain { 0 } else { index });
                        Ok::<_, &str>(Output(&dropped))
                    })
                },
                |_| (),
            );
            assert!(measurement.complete);
            assert_eq!(measurement.dimensions.len(), 3);
            assert!(measurement.first_yield_ns.unwrap() <= measurement.total_ns);
            assert_eq!(dropped.get(), if retain { 0 } else { 3 });
            drop(measurement.retained);
            assert_eq!(dropped.get(), 3);
        }
    }

    #[test]
    fn resources_stops_at_exact_error_without_claiming_completion() {
        for retain in [false, true] {
            for successes in [0, 1] {
                let dropped = Cell::new(0);
                let measurement = consume(
                    3,
                    retain,
                    || {
                        (0..3).map(|index| {
                            assert!(index <= successes, "must not request an item after error");
                            if index == successes {
                                Err("exact diagnostic\nincluding details")
                            } else {
                                Ok(Output(&dropped))
                            }
                        })
                    },
                    |_| (),
                );
                assert!(!measurement.complete);
                assert_eq!(
                    measurement.error,
                    Some("exact diagnostic\nincluding details")
                );
                assert_eq!(measurement.dimensions.len(), successes);
                assert!(measurement.first_yield_ns.is_some());
                assert_eq!(dropped.get(), if retain { 0 } else { successes });
                drop(measurement.retained);
                assert_eq!(dropped.get(), successes);
            }
        }
    }

    #[test]
    fn resources_empty_or_short_sequence_is_incomplete() {
        for expected in [0, 1] {
            let measurement = consume(expected, false, std::iter::empty::<Result<(), ()>>, |_| ());
            assert!(!measurement.complete);
            assert_eq!(measurement.first_yield_ns, None);
        }
        let measurement = consume(2, false, || std::iter::once(Ok::<_, ()>(())), |_| ());
        assert!(!measurement.complete);
        assert_eq!(measurement.dimensions.len(), 1);
    }

    #[test]
    fn resources_controls_require_explicit_inputs_and_positive_budget() {
        let args = |text: &str| {
            text.split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            controls(&args("stream 4 json case.json")).unwrap().1.get(),
            4
        );
        assert!(
            controls(&args("retain 1 env krakenc case.env other.flp"))
                .unwrap()
                .0
        );
        for invalid in [
            "",
            "stream 0 json case.json",
            "stream 1 env kraken case.env",
            "stream 1 env other case.env case.flp",
            "other 1 json case.json",
            "stream 1 json case.json extra.flp",
        ] {
            assert!(controls(&args(invalid)).is_err());
        }
    }
}
