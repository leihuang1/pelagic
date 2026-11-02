//! Solver-only measurements and complete round-trip-float Debug snapshots.
//! Parsing, formatting, snapshot I/O and destruction are outside each timer.
use std::{
    fmt::Debug,
    hint::black_box,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::Instant,
};

fn measure<T: Debug>(
    warmup: usize,
    reps: usize,
    snapshot: Option<&Path>,
    mut solve: impl FnMut() -> Result<T, String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut samples_ns = Vec::with_capacity(reps);
    let mut last = None;
    for iteration in 0..warmup + reps {
        drop(last.take());
        let start = Instant::now();
        let result = black_box(solve());
        let ns = start.elapsed().as_nanos();
        match result {
            Ok(result) => last = Some(result),
            Err(error) => {
                println!(
                    "{}",
                    serde_json::json!({"status": "solve-error", "error": error})
                );
                return Ok(());
            }
        }
        if iteration >= warmup {
            samples_ns.push(ns);
        }
    }
    if let Some(path) = snapshot {
        std::fs::write(path, format!("{:?}", last.as_ref().unwrap()))?;
    }
    println!(
        "{}",
        serde_json::json!({"status": "ok", "samples_ns": samples_ns})
    );
    Ok(())
}

fn load_error(error: impl Debug) {
    println!(
        "{}",
        serde_json::json!({"status": "load-error", "error": format!("{error:?}")})
    );
}

fn field_path(model: &str, input: &Path, explicit: Option<&str>) -> Result<PathBuf, &'static str> {
    if let Some(path) = explicit {
        if !matches!(model, "kraken" | "modal") || input.extension().is_none_or(|e| e != "env") {
            return Err("explicit FLP requires a KRAKEN legacy ENV input");
        }
        if path.is_empty() {
            return Err("explicit FLP path must not be empty");
        }
        Ok(PathBuf::from(path))
    } else {
        Ok(input.with_extension("flp"))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if !(6..=9).contains(&args.len()) {
        return Err("usage: pelagic-perf {kraken|modal|bellhop} INPUT THREADS WARMUP REPS [SNAPSHOT|-] [kraken|krakenc] [FLP]".into());
    }
    let path = Path::new(&args[2]);
    let flp = field_path(&args[1], path, args.get(8).map(String::as_str))?;
    let threads: NonZeroUsize = args[3].parse()?;
    let warmup: usize = args[4].parse()?;
    let reps: NonZeroUsize = args[5].parse()?;
    warmup
        .checked_add(reps.get())
        .ok_or("iteration count overflow")?;
    let snapshot = args.get(6).filter(|s| *s != "-").map(Path::new);
    let engine = match args.get(7).map(String::as_str).unwrap_or("kraken") {
        "kraken" => kraken::ModeSolver::Kraken,
        "krakenc" => kraken::ModeSolver::Krakenc,
        _ => return Err("unknown KRAKEN engine".into()),
    };
    match args[1].as_str() {
        "kraken" => {
            let cases = if path.extension().is_some_and(|e| e == "json") {
                kraken::json::decode_case_document(&std::fs::read(path)?[..], path)
                    .and_then(|document| document.cases(path))
            } else {
                kraken::legacy::load_field_cases(path, &flp, engine)
            };
            match cases {
                Ok(cases) => measure(warmup, reps.get(), snapshot, || {
                    kraken::parallelism::with_threads(threads, || {
                        kraken::solve_frequencies(&cases).collect::<Result<Vec<_>, _>>()
                    })
                    .map_err(|error| format!("{error:?}"))
                })?,
                Err(error) => load_error(error),
            }
        }
        "modal" => match kraken::legacy::load_frequency_cases(path, &flp, engine) {
            Ok(cases) => measure(warmup, reps.get(), snapshot, || {
                kraken::parallelism::with_threads(threads, || {
                    cases
                        .iter()
                        .map(kraken::solve)
                        .collect::<Result<Vec<_>, _>>()
                })
                .map_err(|error| format!("{error:?}"))
            })?,
            Err(error) => load_error(error),
        },
        "bellhop" => {
            let case = if path.extension().is_some_and(|e| e == "json") {
                bellhop::json::load_case_document(&std::fs::read(path)?[..])
                    .map_err(|error| format!("{error:?}"))
            } else {
                bellhop::legacy::load_case(path).map_err(|error| format!("{error:?}"))
            };
            match case {
                Ok(case) => measure(warmup, reps.get(), snapshot, || {
                    bellhop::parallelism::with_threads(threads, || {
                        bellhop::solver::run(
                            &case.value,
                            bellhop::solver::SimulationLimits::default(),
                        )
                    })
                    .map_err(|error| format!("{error:?}"))
                })?,
                Err(error) => load_error(error),
            }
        }
        _ => return Err("unknown model".into()),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_path_preserves_defaults_and_validates_explicit_flp() {
        let env = Path::new("environment.env");
        for model in ["kraken", "modal"] {
            assert_eq!(
                field_path(model, env, None).unwrap(),
                Path::new("environment.flp")
            );
            assert_eq!(
                field_path(model, env, Some("other/geometry.flp")).unwrap(),
                Path::new("other/geometry.flp")
            );
            assert!(field_path(model, Path::new("case.json"), Some("geometry.flp")).is_err());
            assert!(field_path(model, env, Some("")).is_err());
        }
        assert!(field_path("bellhop", env, Some("geometry.flp")).is_err());
        assert!(field_path("bellhop", env, None).is_ok());
    }

    #[test]
    fn warmup_repeats_and_snapshot_keep_signed_zero() {
        let path =
            std::env::temp_dir().join(format!("pelagic-perf-{}.snapshot", std::process::id()));
        let mut calls = 0;
        measure(2, 3, Some(&path), || {
            calls += 1;
            Ok(vec![-0.0_f64, f64::from_bits(1), 1.0])
        })
        .unwrap();
        assert_eq!(calls, 5);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[-0.0, 5e-324, 1.0]"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn errors_stop_without_publishing_a_snapshot() {
        let path = std::env::temp_dir().join(format!(
            "pelagic-perf-error-{}.snapshot",
            std::process::id()
        ));
        let mut calls = 0;
        measure::<()>(0, 3, Some(&path), || {
            calls += 1;
            Err("expected diagnostic".into())
        })
        .unwrap();
        assert_eq!(calls, 1);
        assert!(!path.exists());
    }
}
