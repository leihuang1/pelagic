# Fortran to Rust porting map

| Reference | Rust | Status |
|---|---|---|
| `Bellhop/ReadEnvironmentBell.f90::ReadEnvironment` | `bellhop::legacy::env` | Parser verified |
| `Bellhop/ReadEnvironmentBell.f90::ReadTopOpt` | `bellhop::legacy::env` | Parser verified |
| `misc/SourceReceiverPositions.f90` | `bellhop::legacy::env` | Parser verified |
| `Bellhop/angleMod.f90::ReadRayElevationAngles` | `bellhop::legacy::env` | Parser verified |
| `misc/subtabulate.f90` | `bellhop::legacy::env` | Parser verified |
| `Bellhop/sspMod.f90::Quad` input loading | `bellhop::legacy::auxiliary` | Parser verified |
| `Bellhop/bdryMod.f90::ReadATI` | `bellhop::legacy::auxiliary` | Parser verified |
| `Bellhop/bdryMod.f90::ReadBTY` | `bellhop::legacy::auxiliary` | Parser verified |
| `misc/RefCoef.f90::ReadReflectionCoefficient` (`.brc`/`.trc`/`.irc`) | `bellhop::legacy::auxiliary` | Parser verified |
| `misc/RefCoef.f90::InterpolateIRC` and `Kraken/bounce.f90` reflection formula | `bellhop::solver::reflection::interpolate_internal_reflection` | BOUNCE golden verified |
| `misc/beampattern.f90::ReadPat` | `bellhop::legacy::auxiliary` | Parser verified |
| `Bellhop/Step.f90::Step2D` | `bellhop::solver::integrator::step_2d` | Golden verified |
| `Bellhop/Step.f90::ReduceStep2D` | `bellhop::solver::integrator::reduce_step` | Golden verified |
| `Bellhop/sspMod.f90` `N/C/P/S/Q/A` models | `bellhop::solver::ssp` | Golden verified |
| `Bellhop/bdryMod.f90::ComputeBdryTangentNormal` | `bellhop::solver::boundary` | Differentially exercised |
| `bellhop.f90::TraceRay2D` | `bellhop::solver::trace_ray` | Golden verified |
| `bellhop.f90::Reflect2D` | `bellhop::solver::reflection::reflect_2d` | Differentially exercised |
| `Bellhop/WriteRay.f90` | `bellhop::result::{RayTrajectory, RayPoint}` | In-memory equivalent; legacy writer intentionally omitted |
| `Bellhop/influence.f90::InfluenceGeoHatCart` | `bellhop::solver::influence::geo_hat_cartesian` | Golden verified |
| `Bellhop/influence.f90::InfluenceGeoGaussianCart` | `bellhop::solver::influence::geo_gaussian_cartesian` | Golden verified |
| `Bellhop/influence.f90::InfluenceGeoHatRayCen` | `bellhop::solver::influence::geo_hat_ray_centered` | Golden verified |
| `Bellhop/influence.f90::InfluenceSGB` | `bellhop::solver::influence::simple_gaussian` | Golden verified |
| `Bellhop/influence.f90::InfluenceCervenyCart` | `bellhop::solver::influence::cerveny_cartesian` | Golden verified |
| `Bellhop/influence.f90::InfluenceCervenyRayCen` | `bellhop::solver::influence::cerveny_ray_centered` | Golden verified |
| `bellhop.f90::PickEpsilon` | `bellhop::solver::influence::pick_epsilon` | Differentially exercised for `F/M/W` |
| `Bellhop/influence.f90::{BranchCut,Hermite}` | `bellhop::solver::influence::{branch_cut,hermite}` | Golden verified |
| `Bellhop/influence.f90::{ApplyContribution,ScalePressure}` | `bellhop::solver::influence::{apply_contribution,scale_pressure}` | Golden verified |
| `Bellhop/ArrMod.f90::AddArr` | `bellhop::solver::influence::add_arrival` | Golden verified |
| `Kraken/BCImpedance{c}Mod.f90::{ElasticUP,ElasticDN}` | `kraken::solver::elastic::{SolidMesh,cap_impedance}` | Homogeneous/depth-varying finite caps and contiguous fluid stacks verified through both engines, FIELD and CLI-HDF5 |
| `misc/sspMod.f90::n2Linear` complex reciprocal interpolation | `kraken::solver::profile::Profile::complex_speed` | Reciprocal-before-weighting; wide three-layer KRAKENC naturally searches 5 then 4, complete legacy/JSON products pass |
| `Kraken/kraken{c}.f90::Solve` spectral reduction/Richardson history | `kraken::solver::refinement::Refinement::advance` | Surviving first-mesh data retained; original TLslices double passes both engines, FIELD and legacy/JSON CLI-HDF5 |
| `misc/RootFinderSecantMod.f90::ZSecantCX` | `kraken::solver::complex_modes::{solve,secant}` | Unchanged secant/deflation; lossless-fluid spacing prediction bounded by bottom cp enables full original BroadBand/MunkK at unchanged 300M ceiling |
| `Kraken/kraken.f90::Solve2` finite-solid real search | `kraken::solver::modes::Mesh::solid_roots` | Deflated secants/raw-root Neville and ordered M search bound verified for single/multi-fluid finite elasticity |
| `Kraken/BCImpedance{c}Mod.f90` elastic A half-space formula | `kraken::solver::elastic::half_space` | KRAKEN/KRAKENC top+bottom, full modes/FIELD/CLI-HDF5 differential verified |
| `Kraken/kraken.f90::{FUNCT,AcousticLayers,Solve1,Bisection}` elastic mode count/isolation | `kraken::solver::modes::Mesh::elastic_count_lanes` | Verified real top/bottom; shared intervals, ZBRENTX and subsequent non-deflated Solve2 |
| `Kraken/RootFinderBrent.f90::ZBRENTX` extended-range real root refinement | `kraken::solver::modes::brent` | Pinned exponents, sequential assignments and same-sign initialized-root history retained |
| `Kraken/kraken{c}.f90::Normalize` half-space admittance derivative | `kraken::solver::{modes,complex_modes}` | Real/complex elastic half-space normalization verified; pinned group/loss limitations retained |
| `KrakenField/EvaluateMod.f90::Evaluate` | `kraken::solver::synthesize_field` | Single-profile X/R/S and coherent/incoherent addition verified |
| `misc/beampattern.f90::ReadPat` + `interp1` | `kraken::legacy` + `kraken::solver` | SBP parsing, dB conversion and take-off-angle shading verified |
| `KrakenField/EvaluateADMod.f90` | `kraken::solver::field::adiabatic` | Full original Gulf AD modes/FIELD/CLI-HDF5 differential verified |
| `KrakenField/EvaluateCMMod.f90::{EvaluateCM,PLeft,NewProfile,CalculateTail}` | `kraken::solver::field::{coupled,project}` | Full original Gulf CM and changing-depth derivatives verified |
| `ReadEnvironment` repeated profile records + FIELD `rProf` | `kraken::legacy::load_field_cases` + `FieldCase` | Immutable profile/frequency sequences with cumulative bounds |
