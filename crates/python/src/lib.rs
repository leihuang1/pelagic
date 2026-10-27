//! Thin in-process adapter. Acoustic validation and all numerical work remain in Rust solvers.
mod arrays;

use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyModule};
use sha2::{Digest, Sha256};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pyo3::create_exception!(_core, NativeError, PyException);

fn failure(kind: &str, diagnostics: impl serde::Serialize) -> PyErr {
    NativeError::new_err(serde_json::json!({"kind":kind, "diagnostics": diagnostics}).to_string())
}
fn adapter_error(kind: &str, code: &str, field: &str, message: impl std::fmt::Display) -> PyErr {
    failure(
        kind,
        serde_json::json!([{"code": code, "severity":"error", "field":field,
        "message": message.to_string(), "source": null}]),
    )
}
fn bh_error(kind: &str, report: &bellhop::DiagnosticReport) -> PyErr {
    failure(kind, report.diagnostics())
}
fn kr_error(kind: &str, report: &kraken::DiagnosticReport) -> PyErr {
    failure(kind, report.diagnostics())
}

#[pyclass(frozen, module = "pelagic._core")]
struct Cancellation {
    token: Arc<AtomicBool>,
}
#[pymethods]
impl Cancellation {
    #[new]
    fn new() -> Self {
        Self {
            token: Arc::new(AtomicBool::new(false)),
        }
    }
    fn cancel(&self) {
        self.token.store(true, Ordering::Relaxed);
    }
}

enum CaseData {
    Bellhop(Box<bellhop::Case>),
    Kraken(Vec<kraken::FieldCase>),
}
#[pyclass(frozen, module = "pelagic._core")]
struct NativeCase {
    data: Arc<CaseData>,
    document: String,
    input_sha256: String,
    warnings: String,
    inputs: Vec<PathBuf>,
}

impl NativeCase {
    fn parse(model: &str, document: &str, inputs: Vec<PathBuf>) -> PyResult<Self> {
        let (data, warnings, canonical) = match model {
            "bellhop" => {
                let outcome = bellhop::json::load_case_document(document.as_bytes())
                    .map_err(|e| bh_error("validation", e.report()))?;
                let doc = bellhop::json::export_case_document(&outcome.value)
                    .map_err(|r| bh_error("validation", &r))?;
                (
                    CaseData::Bellhop(Box::new(outcome.value)),
                    serde_json::to_string(&outcome.warnings),
                    serde_json::to_string(&doc),
                )
            }
            "kraken" => {
                let path = std::path::Path::new("request.json");
                let doc = kraken::json::decode_case_document(document.as_bytes(), path)
                    .map_err(|r| kr_error("validation", &r))?;
                let cases = doc.cases(path).map_err(|r| kr_error("validation", &r))?;
                // to_dict() must reload: reject a case whose JSON form outgrows the input limit.
                let canonical = kraken::json::encode_case_document(&doc, path)
                    .map_err(|r| kr_error("validation", &r))?;
                (CaseData::Kraken(cases), Ok("[]".into()), Ok(canonical))
            }
            _ => {
                return Err(adapter_error(
                    "validation",
                    "PG0201",
                    "model",
                    "unknown model",
                ));
            }
        };
        let encode = |e| adapter_error("validation", "PG0103", "json", e);
        Ok(Self {
            data: Arc::new(data),
            input_sha256: format!("{:x}", Sha256::digest(document.as_bytes())),
            document: canonical.map_err(encode)?,
            warnings: warnings.map_err(encode)?,
            inputs,
        })
    }
}

#[pymethods]
impl NativeCase {
    #[staticmethod]
    fn from_json(model: &str, document: &str) -> PyResult<Self> {
        Self::parse(model, document, vec![])
    }

    #[staticmethod]
    #[pyo3(signature = (model, path, flp=None, solver=None))]
    fn load(
        py: Python<'_>,
        model: String,
        path: PathBuf,
        flp: Option<PathBuf>,
        solver: Option<String>,
    ) -> PyResult<Self> {
        py.detach(move || {
            let json = path.extension().is_some_and(|e| e == "json");
            if json {
                if flp.is_some() {
                    return Err(adapter_error(
                        "validation",
                        "KR0202",
                        "flp",
                        "--flp is invalid for JSON",
                    ));
                }
                let document = read_json(&model, &path)?;
                let case = Self::parse(&model, &document, vec![absolute(&path)?])?;
                if let (Some(solver), CaseData::Kraken(cases)) =
                    (solver.as_deref(), case.data.as_ref())
                    && engine(solver)? != cases[0].profiles()[0].mode_solver
                {
                    return Err(adapter_error(
                        "validation",
                        "KR0202",
                        "mode_solver",
                        "solver must match JSON",
                    ));
                }
                return Ok(case);
            }
            match model.as_str() {
                "bellhop" => {
                    let (outcome, _, paths) = bellhop::legacy::load_case_with_inputs(&path)
                        .map_err(|r| bh_error("validation", &r))?
                        .into_parts();
                    let doc = bellhop::json::export_case_document(&outcome.value)
                        .map_err(|r| bh_error("validation", &r))?;
                    let text = serde_json::to_string(&doc)
                        .map_err(|e| adapter_error("validation", "BH0103", "json", e))?;
                    let mut case = Self::parse(
                        "bellhop",
                        &text,
                        paths.iter().map(|p| absolute(p)).collect::<PyResult<_>>()?,
                    )?;
                    // Legacy warnings are input evidence even when canonical JSON has removed that legacy syntax.
                    case.warnings = serde_json::to_string(&outcome.warnings)
                        .map_err(|e| adapter_error("validation", "BH0103", "json", e))?;
                    Ok(case)
                }
                "kraken" => {
                    let flp = flp.unwrap_or_else(|| path.with_extension("flp"));
                    let input = kraken::input::load_legacy(
                        &path,
                        &flp,
                        engine(solver.as_deref().unwrap_or("kraken"))?,
                    )
                    .map_err(|r| kr_error("validation", &r))?;
                    let document = input.document().map_err(|r| kr_error("validation", &r))?;
                    let text = serde_json::to_string(&document)
                        .map_err(|e| adapter_error("validation", "KR0103", "json", e))?;
                    Self::parse(
                        "kraken",
                        &text,
                        input
                            .snapshots()
                            .iter()
                            .map(|s| absolute(s.path()))
                            .collect::<PyResult<_>>()?,
                    )
                }
                _ => Err(adapter_error(
                    "validation",
                    "PG0201",
                    "model",
                    "unknown model",
                )),
            }
        })
    }

    #[getter]
    fn document(&self) -> &str {
        &self.document
    }
    #[getter]
    fn warnings(&self) -> &str {
        &self.warnings
    }

    fn run(
        &self,
        py: Python<'_>,
        cancellation: &Cancellation,
        threads: NonZeroUsize,
    ) -> PyResult<NativeResult> {
        let token = cancellation.token.clone();
        let solve = || match self.data.as_ref() {
            CaseData::Bellhop(case) => bellhop::cancellation::with_token(token, || {
                bellhop::solver::run(case, bellhop::solver::SimulationLimits::default())
                    .map(ResultData::Bellhop)
                    .map_err(|r| bh_error("simulation", &r))
            }),
            CaseData::Kraken(cases) => kraken::cancellation::with_token(token, || {
                let mut results = Vec::with_capacity(cases.len());
                for (index, result) in kraken::solve_frequencies(cases).enumerate() {
                    results.push(result.map_err(|r| {
                        let diagnostics: Vec<_> = r
                            .diagnostics()
                            .iter()
                            .cloned()
                            .map(|mut d| {
                                d.field = format!("frequencies[{index}].{}", d.field);
                                d
                            })
                            .collect();
                        failure("simulation", diagnostics)
                    })?);
                }
                Ok(ResultData::Kraken(results))
            }),
        };
        let data = py.detach(|| {
            bellhop::parallelism::with_threads(threads, || {
                kraken::parallelism::with_threads(threads, solve)
            })
        })?;
        Ok(NativeResult {
            case: self.data.clone(),
            data,
            document: self.document.clone(),
            input_sha256: self.input_sha256.clone(),
            warnings: self.warnings.clone(),
            inputs: self.inputs.clone(),
        })
    }
}

fn engine(name: &str) -> PyResult<kraken::ModeSolver> {
    match name {
        "kraken" => Ok(kraken::ModeSolver::Kraken),
        "krakenc" => Ok(kraken::ModeSolver::Krakenc),
        _ => Err(adapter_error(
            "validation",
            "KR0202",
            "mode_solver",
            "expected kraken or krakenc",
        )),
    }
}
fn absolute(path: &Path) -> PyResult<PathBuf> {
    std::path::absolute(path).map_err(|e| adapter_error("validation", "PG0001", "input", e))
}
fn read_json(model: &str, path: &Path) -> PyResult<String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| {
        adapter_error(
            "validation",
            if model == "kraken" {
                "KR0001"
            } else {
                "BH0001"
            },
            "input",
            e,
        )
    })?;
    let mut text = String::new();
    file.take(if model == "kraken" {
        kraken::legacy::MAX_INPUT_BYTES + 1
    } else {
        u64::MAX
    })
    .read_to_string(&mut text)
    .map_err(|e| adapter_error("validation", "PG0001", "input", e))?;
    Ok(text)
}

enum ResultData {
    Bellhop(bellhop::result::SimulationResult),
    Kraken(Vec<kraken::ProfileSimulationResult>),
}
#[pyclass(frozen, module = "pelagic._core")]
struct NativeResult {
    case: Arc<CaseData>,
    data: ResultData,
    document: String,
    input_sha256: String,
    warnings: String,
    inputs: Vec<PathBuf>,
}
#[pymethods]
impl NativeResult {
    fn arrays<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let result = match (&*self.case, &self.data) {
            (CaseData::Bellhop(case), ResultData::Bellhop(result)) => {
                arrays::bellhop(py, case, result)?
            }
            (CaseData::Kraken(cases), ResultData::Kraken(results)) => {
                arrays::kraken(py, cases, results)?
            }
            _ => unreachable!("result is paired with its immutable case"),
        };
        result.set_item("case_json", &self.document)?;
        result.set_item("warnings_json", &self.warnings)?;
        result.set_item("input_sha256", &self.input_sha256)?;
        result.set_item(
            "case_sha256",
            format!("{:x}", Sha256::digest(self.document.as_bytes())),
        )?;
        Ok(result)
    }

    #[pyo3(signature = (directory, max_output_bytes=268_435_456))]
    #[allow(clippy::needless_pass_by_value)] // Python path extraction produces an owned PathBuf.
    fn write_native(
        &self,
        py: Python<'_>,
        directory: PathBuf,
        max_output_bytes: u64,
    ) -> PyResult<String> {
        py.detach(|| {
            let (result, warnings) = native_output::diagnostic::capture(|| native_output::directory::publish(&directory, &self.inputs.iter().map(PathBuf::as_path).collect::<Vec<_>>(), Some(max_output_bytes), |scratch| -> Result<(), String> {
                match (&*self.case, &self.data) {
                    (CaseData::Bellhop(case), ResultData::Bellhop(result)) => native_output::bellhop::write(scratch, "case", case, result)?,
                    (CaseData::Kraken(cases), ResultData::Kraken(results)) => {
                        for (index, (case, result)) in cases.iter().zip(results).enumerate() {
                            let stem = if cases.len() == 1 { "case".to_owned() } else { format!("case.f{index:04}") };
                            native_output::kraken::write(scratch, &stem, case, result, max_output_bytes)?;
                        }
                    }
                    _ => unreachable!("result is paired with its immutable case"),
                }
                std::fs::write(scratch.join("case.json"), &self.document).map_err(|e| e.to_string())?;
                std::fs::write(scratch.join("case.prt"), format!("Pelagic {} native result (not Fortran PRT)\nreference: Acoustics Toolbox v2023.5\ninput_sha256: {}\ncase_sha256: {:x}\nwarnings: {}\n", env!("CARGO_PKG_VERSION"), self.input_sha256, Sha256::digest(self.document.as_bytes()), self.warnings)).map_err(|e| e.to_string())
            }));
            if let Err(error) = result {
                let mut items = vec![serde_json::json!({"code":"PG0402", "severity":"error", "field":"output", "message":error, "source":null})];
                for warning in warnings {
                    items.push(serde_json::to_value(warning).map_err(|e| adapter_error("output", "PG0402", "output", e))?);
                }
                return Err(failure("output", items));
            }
            serde_json::to_string(&warnings).map_err(|e| adapter_error("output", "PG0402", "output", e))
        })
    }
}

/// CPUs available to this process, honouring affinity and container quotas.
#[pyfunction]
fn available_threads() -> usize {
    std::thread::available_parallelism().map_or(1, NonZeroUsize::get)
}

#[pymodule]
fn _core(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(available_threads, module)?)?;
    module.add_class::<Cancellation>()?;
    module.add_class::<NativeCase>()?;
    module.add_class::<NativeResult>()?;
    module.add("NativeError", module.py().get_type::<NativeError>())?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    let schemas = PyDict::new(module.py());
    schemas.set_item("bellhop", bellhop::json::SCHEMA_VERSION)?;
    schemas.set_item("kraken", kraken::json::SCHEMA_VERSION)?;
    module.add("case_schema_versions", schemas)?;
    module.add("compatibility_reference", "Acoustics Toolbox v2023.5")?;
    module.add(
        "reference_commit",
        "475108519289c6fb488b58980c644ea14eccc604",
    )?;
    Ok(())
}
