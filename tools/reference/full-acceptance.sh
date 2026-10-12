#!/usr/bin/env bash
# Full pre-release acceptance. Case lists are the former differential-reference CI job.
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"
if [[ $# -gt 1 || ( $# -eq 1 && $1 != --allow-dirty ) ]]; then
  echo "usage: $0 [--allow-dirty]" >&2
  exit 2
fi
diagnostic=$#
sha=$(git rev-parse HEAD)
dirty=$(git status --porcelain)
if [[ -n $dirty && $# -eq 0 ]]; then
  echo 'Release acceptance needs a clean commit. --allow-dirty runs diagnostics, never release acceptance.' >&2
  exit 2
fi
run="$root/target/acceptance/$(date -u +%Y%m%dT%H%M%SZ)-$$"
mkdir -p "$run"
export PELAGIC_REFERENCE_DIR="$run/reference"
mkdir "$PELAGIC_REFERENCE_DIR"
reference_root=${PELAGIC_REFERENCE_DIR#"$root/"}
# External comparator overrides must not silently change acceptance paths/tolerances.
for name in ${!KRAKEN_@} ${!BELLHOP_@}; do unset "$name"; done
export BELLHOP_REFERENCE_IMAGE=pelagic-reference:v2023.5-amd64
# CLI helpers read target/release: never build elsewhere and read a stale binary.
export CARGO_TARGET_DIR="$root/target"
export CARGO_TERM_COLOR=never
export CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true
export CARGO_PROFILE_RELEASE_OVERFLOW_CHECKS=true
labels=()
while IFS= read -r label; do labels+=("$label"); done < <(grep '^step ' "${BASH_SOURCE[0]}" | cut -d "'" -f 2)
statuses=()
times=()
for label in "${labels[@]}"; do statuses+=("NOT RUN"); times+=(0); done
current=-1
started=$SECONDS
step_started=$SECONDS
finish() {
  code=$?
  trap - EXIT
  if (( current >= 0 )); then
    times[current]=$((SECONDS - step_started))
    if (( code == 0 )); then statuses[current]=PASS; else statuses[current]=FAIL; fi
  fi
  if (( code == 0 && current != ${#labels[@]} - 1 )); then code=1; fi
  outcome=FAILED
  if (( code == 0 )); then
    if [[ $diagnostic == 0 && -z $dirty && -z $(git status --porcelain) && $(git rev-parse HEAD) == "$sha" ]]; then
      outcome='PASS — complete acceptance of the recorded clean commit'
    else
      outcome='NOT RELEASE ACCEPTANCE — numerical checks passed in diagnostic mode or on a dirty/changed tree'
      code=2
    fi
  fi
  {
    printf '# Full numerical acceptance\n\n%s\n\n' "$outcome"
    printf -- '- Commit: `%s`\n- Elapsed: %ss\n- Initial dirty tree: %s\n' "$sha" "$((SECONDS - started))" "${dirty:-none}"
    printf -- '- Final commit: `%s`\n- Final dirty tree: %s\n' "$(git rev-parse HEAD)" "$(git status --porcelain)"
    printf '\nSystem/toolchain and oracle build: [run.log](run.log).\n'
    printf 'Every invoked case, engine, named test and result is in that log; fresh products remain in `reference/`.\n\n'
    printf '| Stage | Result | Seconds |\n|---|---|---:|\n'
    for ((i=0; i<${#labels[@]}; i++)); do
      printf '| %s | %s | %s |\n' "${labels[i]}" "${statuses[i]}" "${times[i]}"
    done
    printf '\nA failed stage may have a completed prefix: only its logged passing cases passed; the rest and all NOT RUN stages are unvalidated.\n'
    printf 'Before release, manually match this clean SHA to the release commit and the Windows ZIP CI result. A focused rerun is not full acceptance.\n'
  } > "$run/report.md"
  echo "Report: $run/report.md"
  exit "$code"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
exec > >(tee "$run/run.log") 2>&1
step() {
  if (( current >= 0 )); then
    statuses[current]=PASS
    times[current]=$((SECONDS - step_started))
  fi
  current=$((current + 1))
  [[ $1 == "${labels[current]}" ]]
  statuses[current]=RUNNING
  step_started=$SECONDS
  printf '\n=== %s ===\n' "$1"
}
set -x
step 'Toolchain and committed references'
uname -a
rustc -Vv
cargo -V
expected=$(awk -F '"' '/^channel = / { print $2 }' rust-toolchain.toml)
[[ $(rustc --version) == "rustc $expected "* ]]
printf 'RUSTFLAGS=%s\nRUSTDOCFLAGS=%s\n' "${RUSTFLAGS:-}" "${RUSTDOCFLAGS:-}"
python3 tools/reference/verify-goldens.py
step 'Ordinary regression and development checks'
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features
RUSTDOCFLAGS='-D warnings' cargo doc --locked --workspace --all-features --no-deps

step 'Build pinned Fortran reference image'
tools/reference/build-image.sh

step 'Prepare official differential cases'
mkdir -p $reference_root/cases
docker run --rm --platform linux/amd64 --volume "$PWD/$reference_root/cases:/out" \
  --entrypoint /bin/sh pelagic-reference:v2023.5-amd64 \
  -c "cp /opt/acoustics-toolbox/tests/Munk/MunkB_Coh.env /out/ && \
      cp /opt/acoustics-toolbox/tests/SBCX/sbcx_Arr_asc.env /out/ && \
      cp /opt/acoustics-toolbox/tests/Munk/MunkK.env /opt/acoustics-toolbox/tests/Munk/MunkK.flp /out/ && \
      cp /opt/acoustics-toolbox/tests/Munk/MunkAnalytic.env /opt/acoustics-toolbox/tests/Munk/MunkS.env /out/ && \
      cp /opt/acoustics-toolbox/tests/MunkLeaky/MunkKleaky.env /opt/acoustics-toolbox/tests/MunkLeaky/MunkKleaky.flp /out/ && \
      cp /opt/acoustics-toolbox/tests/MunkLeaky/MunkKwb.env /opt/acoustics-toolbox/tests/MunkLeaky/MunkKwb.flp /out/ && \
      cp /opt/acoustics-toolbox/tests/MunkLeaky/MunkKbb.env /opt/acoustics-toolbox/tests/MunkLeaky/MunkKbb.flp /out/ && \
      cp /opt/acoustics-toolbox/tests/sduct/sductK.env /opt/acoustics-toolbox/tests/sduct/sductK.flp /out/ && \
      cp /opt/acoustics-toolbox/tests/calib/calibK.env /opt/acoustics-toolbox/tests/calib/calibK.flp /out/ && \
      mkdir -p /out/BroadBand && \
      cp /opt/acoustics-toolbox/tests/BroadBand/MunkK.env /opt/acoustics-toolbox/tests/BroadBand/MunkK.flp /out/BroadBand/ && \
      mkdir -p /out/Gulf && \
      cp /opt/acoustics-toolbox/tests/Gulf/gulf_rd.env /opt/acoustics-toolbox/tests/Gulf/gulf_ad.flp /opt/acoustics-toolbox/tests/Gulf/gulf_cm.flp /out/Gulf/"

step 'Prepare pinned original MunkK modes and FIELD'
tools/reference/run-kraken-case.sh kraken $reference_root/cases/MunkK.env

step 'Fluid KRAKENC modes and FIELD numerical differential'
for case in PekerisComplex PekerisComplexBlank PekerisComplexSlow PekerisComplexCLow PekerisComplexRefined PekerisComplexGradient PekerisComplexReverseGradient MunkLeakyPartial MunkLeakyPartialLoss MunkLeakyPartialC MunkLeakyPartialP MunkLeakyPartialS PekerisComplexSpline3 MunkAnalyticComplex TabRefBrcN TabRefBrcC TabRefIrcN TabRefIrcC; do
  tools/reference/run-kraken-case.sh krakenc "crates/kraken/tests/fixtures/$case.env"
  KRAKEN_COMPLEX_CASE="$case" \
  KRAKEN_COMPLEX_REFERENCE_ROOT="$PWD/$reference_root/$case-krakenc/$case" \
  tools/reference/test-one.sh --release -p kraken --test differential_reference \
    complex_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done
# Compare unmodified original .env/.flp pairs to fresh .mod/.prt/.shd.
for case in MunkKleaky MunkKwb MunkKbb sductK calibK; do
  tools/reference/run-kraken-case.sh krakenc "$reference_root/cases/$case.env"
  KRAKEN_ORIGINAL_COMPLEX_ENV="$PWD/$reference_root/cases/$case.env" \
  KRAKEN_ORIGINAL_COMPLEX_ROOT="$PWD/$reference_root/$case-krakenc/$case" \
    tools/reference/test-one.sh --release -p kraken --test differential_reference \
      complex_original_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done

step 'Fluid modes and FIELD numerical differential'
for case in Pekeris PekerisFiltered PekerisDense PekerisDenseLoss PekerisRefined PekerisSpline3 PekerisRigid PekerisRigidLoss PekerisHard PekerisHardBoth PekerisRigidPlane MunkLossless MunkBottomLoss MunkAnalytic SductTrapped SductPchip SductSpline; do
  tools/reference/compare-kraken.sh "crates/kraken/tests/fixtures/$case.env"
done
# Upstream MunkAnalytic.flp is a three-line legacy example that v2023.5 FIELD cannot parse.
# Keep its .env byte-for-byte original; use a separately labelled coherent FIELD input.
cp crates/kraken/tests/fixtures/MunkAnalytic.flp $reference_root/cases/MunkAnalytic.flp
tools/reference/compare-kraken.sh $reference_root/cases/MunkAnalytic.env
KRAKEN_DIFFERENTIAL_ENV="$PWD/$reference_root/cases/MunkK.env" \
KRAKEN_DIFFERENTIAL_ROOT="$PWD/$reference_root/MunkK-kraken/MunkK" \
tools/reference/test-one.sh --release -p kraken --test differential_reference \
  fluid_matches_fresh_reference -- --ignored --exact --nocapture

step 'Single-profile FIELD extensions and actual CLI HDF5 differential'
(cd crates/kraken/tests/fixtures && shasum -a 256 -c golden/single-profile-field.sha256)
cargo build --locked --release -p cli
for mapping in 'FieldScaled kraken' 'FieldPattern krakenc' 'FieldIncoherent krakenc'; do
  set -- $mapping
  case=$1
  engine=$2
  tools/reference/compare-kraken-hdf5.sh "$engine" "crates/kraken/tests/fixtures/$case.env"
done

step 'Multi-profile adiabatic and coupled FIELD plus actual CLI HDF5'
(cd crates/kraken/tests/fixtures && shasum -a 256 -c golden/multi-profile.sha256)
for input in gulf_rd.env gulf_ad.flp gulf_cm.flp; do
  cmp "$reference_root/cases/Gulf/$input" "crates/kraken/tests/fixtures/original/Gulf/$input"
done
mkdir -p $reference_root/profile-cases
for mapping in 'GulfAd gulf_ad' 'GulfCm gulf_cm'; do
  set -- $mapping
  cp crates/kraken/tests/fixtures/original/Gulf/gulf_rd.env "$reference_root/profile-cases/$1.env"
  cp "crates/kraken/tests/fixtures/original/Gulf/$2.flp" "$reference_root/profile-cases/$1.flp"
done
cp crates/kraken/tests/fixtures/ProfilesAd.env "$reference_root/profile-cases/ProfilesAdIncoherent.env"
cp crates/kraken/tests/fixtures/ProfilesAdIncoherent.flp "$reference_root/profile-cases/ProfilesAdIncoherent.flp"
cargo build --locked --release -p cli
for case in ProfilesAd ProfilesCm ProfilesAdIncoherent GulfAd GulfCm; do
  case "$case" in
    Gulf*|ProfilesAdIncoherent) env="$PWD/$reference_root/profile-cases/$case.env" ;;
    *) env="$PWD/crates/kraken/tests/fixtures/$case.env" ;;
  esac
  reference="$PWD/$reference_root/$case-kraken/$case"
  tools/reference/run-kraken-case.sh kraken "$env"
  KRAKEN_PROFILE_ENV="$env" KRAKEN_PROFILE_ROOT="$reference" \
    tools/reference/test-one.sh --release -p kraken --test differential_reference \
      profile_fields_match_fresh_reference -- --ignored --exact --nocapture
  tools/reference/run-cli-kraken.sh "$env" "$(dirname "$reference")/pelagic"
  KRAKEN_HDF5_RESULT="$(dirname "$reference")/pelagic/$case.h5" KRAKEN_PROFILE_ENV="$env" KRAKEN_PROFILE_ROOT="$reference" \
    tools/reference/test-one.sh --release -p kraken --test differential_reference \
      profile_fields_match_fresh_reference -- --ignored --exact --nocapture
done

step 'Multi-frequency modes and FIELD numerical differential'
for case in PekerisBroadband PekerisComplexBroadband MunkLeakyPchipBroadband; do
  engine=kraken
  if [ "$case" != PekerisBroadband ]; then engine=krakenc; fi
  tools/reference/run-kraken-case.sh "$engine" "crates/kraken/tests/fixtures/$case.env"
  KRAKEN_FREQUENCY_SOLVER="$engine" \
  KRAKEN_DIFFERENTIAL_ENV="$PWD/crates/kraken/tests/fixtures/$case.env" \
  KRAKEN_DIFFERENTIAL_ROOT="$PWD/$reference_root/$case-$engine/$case" \
    tools/reference/test-one.sh --release -p kraken --test differential_reference \
      multifrequency_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done
for engine in kraken krakenc; do
  tools/reference/run-kraken-case.sh "$engine" $reference_root/cases/BroadBand/MunkK.env "$reference_root/BroadBand-MunkK-$engine"
  KRAKEN_FREQUENCY_SOLVER="$engine" \
  KRAKEN_DIFFERENTIAL_ENV="$PWD/$reference_root/cases/BroadBand/MunkK.env" \
  KRAKEN_DIFFERENTIAL_ROOT="$PWD/$reference_root/BroadBand-MunkK-$engine/MunkK" \
    tools/reference/test-one.sh --release -p kraken --test differential_reference \
      multifrequency_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done

step 'Water material modes, FIELD and actual CLI HDF5 differential'
cargo build --locked --release -p cli
for engine in kraken krakenc; do
  for case in WaterLossN WaterLossC WaterLossP WaterLossS WaterLossUnitN WaterLossUnitM WaterLossUnitF WaterLossUnitQ WaterLossUnitL WaterLossPower WaterLossThorp WaterLossFg WaterLossBio WaterLossLeaky; do
    if [ "$case" = WaterLossLeaky ] && [ "$engine" = kraken ]; then continue; fi
    tools/reference/compare-kraken-hdf5.sh "$engine" "crates/kraken/tests/fixtures/$case.env"
  done
done

step 'Smooth boundary and TRC modes, FIELD and actual CLI HDF5 differential'
cargo build --locked --release -p cli
for engine in kraken krakenc; do
  for case in FluidBoundaryVV FluidBoundaryVR FluidBoundaryVA FluidBoundaryRV FluidBoundaryRR FluidBoundaryRA FluidBoundaryAV FluidBoundaryAR FluidBoundaryAA FluidRigidPlaneLoss FluidBoundaryAir FluidTrcN FluidTrcC FluidTrcRigid; do
    if [ "$engine" = kraken ] && { [ "$case" = FluidBoundaryAir ] || [[ "$case" == FluidTrc* ]]; }; then continue; fi
    tools/reference/compare-kraken-hdf5.sh "$engine" "crates/kraken/tests/fixtures/$case.env"
  done
done

step 'Layered fluid modes, FIELD and actual CLI HDF5 differential'
(cd crates/kraken/tests/fixtures && shasum -a 256 -c golden/original-double.sha256 && shasum -a 256 -c golden/layered-refinement.sha256)
mkdir -p $reference_root/layered-original-inputs
docker run --rm --platform linux/amd64 --volume "$PWD/$reference_root/layered-original-inputs:/out" \
  --entrypoint /bin/sh pelagic-reference:v2023.5-amd64 \
  -c 'cp /opt/acoustics-toolbox/tests/TLslices/double.env /opt/acoustics-toolbox/tests/TLslices/fieldbat.flp /out/'
cmp $reference_root/layered-original-inputs/double.env crates/kraken/tests/fixtures/OriginalLayeredDouble.env
cmp $reference_root/layered-original-inputs/fieldbat.flp crates/kraken/tests/fixtures/OriginalLayeredDouble.flp
cargo build --locked --release -p cli
for engine in kraken krakenc; do
  for case in LayeredFluidN LayeredFluidC LayeredFluidP LayeredFluidS LayeredBoundaryVV LayeredBoundaryVR LayeredBoundaryRV LayeredBoundaryRR LayeredBoundaryRA LayeredBoundaryAV LayeredBoundaryAR LayeredBoundaryAA LayeredFluidThree LayeredFluidThreeWide LayeredFluidThreeWideRefined LayeredFluidPlane LayeredFluidPower LayeredFluidBio LayeredFluidLeaky LayeredDoubleRefined OriginalLayeredDouble LayeredNormalization LayeredFluidFractional; do
    if [ "$case" = LayeredFluidLeaky ] && [ "$engine" = kraken ]; then continue; fi
    tools/reference/compare-kraken-hdf5.sh "$engine" "crates/kraken/tests/fixtures/$case.env"
  done
done
tools/reference/test-one.sh --release -p cli --test kraken_run \
  layered_failures_preserve_output_and_remove_scratch -- --exact --nocapture

step 'Elastic half-space modes, FIELD and actual CLI HDF5 differential'
(cd crates/kraken/tests/fixtures && shasum -a 256 -c golden/elastic-halfspace.sha256)
mkdir -p $reference_root/elastic-original-inputs
docker run --rm --platform linux/amd64 --volume "$PWD/$reference_root/elastic-original-inputs:/out" \
  --entrypoint /bin/sh pelagic-reference:v2023.5-amd64 \
  -c 'cp /opt/acoustics-toolbox/tests/TLslices/scholte.env /opt/acoustics-toolbox/tests/TLslices/normal.env /opt/acoustics-toolbox/tests/TLslices/flused.env /opt/acoustics-toolbox/tests/TLslices/fieldbat.flp /out/'
for mapping in 'scholte OriginalElasticScholte' 'normal OriginalElasticNormal' 'flused OriginalElasticFlused'; do
  set -- $mapping
  cmp "$reference_root/elastic-original-inputs/$1.env" "crates/kraken/tests/fixtures/$2.env"
  cmp $reference_root/elastic-original-inputs/fieldbat.flp "crates/kraken/tests/fixtures/$2.flp"
done
cargo build --locked --release -p cli
for engine in kraken krakenc; do
  for case in ElasticHalfBottomN ElasticHalfBottomC ElasticHalfBottomP ElasticHalfBottomS ElasticHalfTopN ElasticHalfTopC ElasticHalfTopP ElasticHalfTopS ElasticHalfBothN ElasticHalfBothC ElasticHalfBothP ElasticHalfBothS ElasticHalfLeaky ElasticHalfTopBroadband ElasticHalfPower ElasticHalfShearOnly OriginalElasticScholte OriginalElasticNormal OriginalElasticFlused; do
    if [ "$engine" = kraken ] && [ "$case" = ElasticHalfLeaky ]; then continue; fi
    tools/reference/compare-kraken-hdf5.sh "$engine" "crates/kraken/tests/fixtures/$case.env"
  done
done
tools/reference/test-one.sh --release -p cli --test kraken_run \
  elastic_failures_preserve_output_and_remove_scratch -- --exact --nocapture

step 'Finite elastic modes, FIELD and actual CLI HDF5 differential'
(cd crates/kraken/tests/fixtures && shasum -a 256 -c golden/finite-elastic.sha256 && shasum -a 256 -c golden/graded-elastic.sha256)
mkdir -p $reference_root/finite-elastic-original-inputs
docker run --rm --platform linux/amd64 --volume "$PWD/$reference_root/finite-elastic-original-inputs:/out" \
  --entrypoint /bin/sh pelagic-reference:v2023.5-amd64 \
  -c 'cp /opt/acoustics-toolbox/tests/TLslices/elsed.env /opt/acoustics-toolbox/tests/TLslices/ice.env /opt/acoustics-toolbox/tests/TLslices/fieldbat.flp /out/'
for mapping in 'elsed OriginalElasticSediment' 'ice OriginalElasticIce'; do
  set -- $mapping
  cmp "$reference_root/finite-elastic-original-inputs/$1.env" "crates/kraken/tests/fixtures/$2.env"
  cmp $reference_root/finite-elastic-original-inputs/fieldbat.flp "crates/kraken/tests/fixtures/$2.flp"
done
cargo build --locked --release -p cli
for engine in kraken krakenc; do
  for case in FiniteElasticBothC FiniteElasticBothN FiniteElasticBothP FiniteElasticBothS FiniteElasticBottomC FiniteElasticBottomN FiniteElasticBottomP FiniteElasticBottomS FiniteElasticPower FiniteElasticRigid FiniteElasticShearOnly FiniteElasticStack FiniteElasticTopC FiniteElasticTopN FiniteElasticTopP FiniteElasticTopS FiniteElasticVacuum FiniteSingleIceC FiniteSingleIceP FiniteSingleIceS FiniteSingleSedimentC FiniteSingleSedimentP FiniteSingleSedimentS OriginalElasticIce OriginalElasticSediment GradedElasticTopN GradedElasticTopC GradedElasticTopP GradedElasticTopS GradedElasticBottomN GradedElasticBottomC GradedElasticBottomP GradedElasticBottomS GradedElasticStack GradedElasticPower GradedElasticBio; do
    tools/reference/compare-kraken-hdf5.sh "$engine" "crates/kraken/tests/fixtures/$case.env"
  done
done
tools/reference/compare-kraken-hdf5.sh kraken crates/kraken/tests/fixtures/GradedElasticTopNRefined.env
tools/reference/test-one.sh --release -p cli --test kraken_run \
  finite_elastic_failures_preserve_output_and_remove_scratch -- --exact --nocapture

step 'KRAKEN CLI HDF5 numerical differential'
cargo build --locked --release -p cli
for case in PekerisBroadband PekerisComplexBroadband MunkLeakyPchipBroadband; do
  engine=kraken
  if [ "$case" != PekerisBroadband ]; then engine=krakenc; fi
  output="$PWD/$reference_root/$case-$engine/pelagic/$case.h5"
  tools/reference/run-cli-kraken.sh "crates/kraken/tests/fixtures/$case.env" "$(dirname "$output")" --solver "$engine"
  KRAKEN_HDF5_RESULT="$output" \
  KRAKEN_FREQUENCY_SOLVER="$engine" \
  KRAKEN_DIFFERENTIAL_ENV="$PWD/crates/kraken/tests/fixtures/$case.env" \
  KRAKEN_DIFFERENTIAL_ROOT="$PWD/$reference_root/$case-$engine/$case" \
    tools/reference/test-one.sh --release -p kraken --test differential_reference \
      multifrequency_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done
for case in MunkKleaky MunkKwb MunkKbb sductK calibK; do
  output="$PWD/$reference_root/$case-krakenc/pelagic/$case.h5"
  tools/reference/run-cli-kraken.sh "$reference_root/cases/$case.env" "$(dirname "$output")" --solver krakenc
  KRAKEN_HDF5_RESULT="$output" \
  KRAKEN_ORIGINAL_COMPLEX_ENV="$PWD/$reference_root/cases/$case.env" \
  KRAKEN_ORIGINAL_COMPLEX_ROOT="$PWD/$reference_root/$case-krakenc/$case" \
    tools/reference/test-one.sh --release -p kraken --test differential_reference \
      complex_original_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done
output="$PWD/$reference_root/MunkK-kraken/pelagic/MunkK.h5"
tools/reference/run-cli-kraken.sh $reference_root/cases/MunkK.env "$(dirname "$output")"
KRAKEN_HDF5_RESULT="$output" \
KRAKEN_DIFFERENTIAL_ENV="$PWD/$reference_root/cases/MunkK.env" \
KRAKEN_DIFFERENTIAL_ROOT="$PWD/$reference_root/MunkK-kraken/MunkK" \
  tools/reference/test-one.sh --release -p kraken --test differential_reference \
    fluid_matches_fresh_reference -- --ignored --exact --nocapture
for engine in kraken krakenc; do
  output="$PWD/$reference_root/BroadBand-MunkK-$engine/pelagic/MunkK.h5"
  tools/reference/run-cli-kraken.sh $reference_root/cases/BroadBand/MunkK.env "$(dirname "$output")" --solver "$engine"
  KRAKEN_FREQUENCY_SOLVER="$engine" KRAKEN_HDF5_RESULT="$output" \
  KRAKEN_DIFFERENTIAL_ENV="$PWD/$reference_root/cases/BroadBand/MunkK.env" \
  KRAKEN_DIFFERENTIAL_ROOT="$PWD/$reference_root/BroadBand-MunkK-$engine/MunkK" \
    tools/reference/test-one.sh --release -p kraken --test differential_reference \
      multifrequency_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done

step 'TabRefCoef generated resources, original FIELD and CLI HDF5 differential'
tools/reference/prepare-tabref.sh
for case in TabRefBrcN TabRefBrcC TabRefIrcN TabRefIrcC; do
  output="$PWD/$reference_root/$case-krakenc/pelagic/$case.h5"
  tools/reference/run-cli-kraken.sh "crates/kraken/tests/fixtures/$case.env" "$(dirname "$output")" --solver krakenc
  KRAKEN_HDF5_RESULT="$output" \
  KRAKEN_COMPLEX_CASE="$case" \
  KRAKEN_COMPLEX_REFERENCE_ROOT="$PWD/$reference_root/$case-krakenc/$case" \
    tools/reference/test-one.sh --release -p kraken --test differential_reference \
      complex_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done
for case in neggradC_geo neggradC_brc neggradC_irc; do
  env="$PWD/$reference_root/TabRefCoef/run1/$case.env"
  root="$PWD/$reference_root/TabRefCoef/run1/$case"
  KRAKEN_ORIGINAL_COMPLEX_ENV="$env" KRAKEN_ORIGINAL_COMPLEX_ROOT="$root" \
    tools/reference/test-one.sh --release -p kraken --test differential_reference \
      complex_original_fluid_matches_fresh_reference -- --ignored --exact --nocapture
  tools/reference/run-cli-kraken.sh "$env" "$(dirname "$root")/pelagic-$case" --solver krakenc
  KRAKEN_HDF5_RESULT="$(dirname "$root")/pelagic-$case/$case.h5" \
  KRAKEN_ORIGINAL_COMPLEX_ENV="$env" KRAKEN_ORIGINAL_COMPLEX_ROOT="$root" \
    tools/reference/test-one.sh --release -p kraken --test differential_reference \
      complex_original_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done

step 'KRAKENC cubic and analytic profiles through CLI HDF5'
for case in MunkLeakyPartialP MunkLeakyPartialS PekerisComplexSpline3 MunkAnalyticComplex; do
  output="$PWD/$reference_root/$case-krakenc/pelagic/$case.h5"
  tools/reference/run-cli-kraken.sh "crates/kraken/tests/fixtures/$case.env" "$(dirname "$output")" --solver krakenc
  KRAKEN_HDF5_RESULT="$output" \
  KRAKEN_COMPLEX_CASE="$case" \
  KRAKEN_COMPLEX_REFERENCE_ROOT="$PWD/$reference_root/$case-krakenc/$case" \
    tools/reference/test-one.sh --release -p kraken --test differential_reference \
      complex_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done
# These are unmodified upstream environments plus a DERIVED FIELD input.
# MunkS was a SCOOTER fixture; neither original three-line .flp parses in FIELD.
for case in MunkS MunkAnalytic; do
  cp crates/kraken/tests/fixtures/MunkAnalytic.flp "$reference_root/cases/$case.flp"
  tools/reference/run-kraken-case.sh krakenc "$reference_root/cases/$case.env"
  env="$PWD/$reference_root/cases/$case.env"
  root="$PWD/$reference_root/$case-krakenc/$case"
  KRAKEN_ORIGINAL_COMPLEX_ENV="$env" KRAKEN_ORIGINAL_COMPLEX_ROOT="$root" \
    tools/reference/test-one.sh --release -p kraken --test differential_reference \
      complex_original_fluid_matches_fresh_reference -- --ignored --exact --nocapture
  tools/reference/run-cli-kraken.sh "$env" "$(dirname "$root")/pelagic" --solver krakenc
  KRAKEN_HDF5_RESULT="$(dirname "$root")/pelagic/$case.h5" \
  KRAKEN_ORIGINAL_COMPLEX_ENV="$env" KRAKEN_ORIGINAL_COMPLEX_ROOT="$root" \
    tools/reference/test-one.sh --release -p kraken --test differential_reference \
      complex_original_fluid_matches_fresh_reference -- --ignored --exact --nocapture
done
KRAKEN_DIFFERENTIAL_ENV="$PWD/$reference_root/cases/BroadBand/MunkK.env" \
  tools/reference/test-one.sh --release -p cli --test kraken_run \
    original_complex_spline_broadband_is_complete_and_preserves_late_failure -- --ignored --exact --nocapture

step 'Self-contained KRAKEN JSON API and actual CLI HDF5 differential'
cargo build --locked --release -p cli
mkdir -p $reference_root/json
for mapping in \
  'WaterLossPower kraken crates/kraken/tests/fixtures/WaterLossPower.env $reference_root/WaterLossPower-kraken/WaterLossPower' \
  'FieldPattern krakenc crates/kraken/tests/fixtures/FieldPattern.env $reference_root/FieldPattern-krakenc/FieldPattern' \
  'FluidTrcC krakenc crates/kraken/tests/fixtures/FluidTrcC.env $reference_root/FluidTrcC-krakenc/FluidTrcC' \
  'LayeredFluidPower krakenc crates/kraken/tests/fixtures/LayeredFluidPower.env $reference_root/LayeredFluidPower-krakenc/LayeredFluidPower' \
  'LayeredFluidThreeWideRefined krakenc crates/kraken/tests/fixtures/LayeredFluidThreeWideRefined.env $reference_root/LayeredFluidThreeWideRefined-krakenc/LayeredFluidThreeWideRefined' \
  'OriginalDoubleKraken kraken crates/kraken/tests/fixtures/OriginalLayeredDouble.env $reference_root/OriginalLayeredDouble-kraken/OriginalLayeredDouble' \
  'OriginalDoubleKrakenc krakenc crates/kraken/tests/fixtures/OriginalLayeredDouble.env $reference_root/OriginalLayeredDouble-krakenc/OriginalLayeredDouble' \
  'FiniteElasticStack krakenc crates/kraken/tests/fixtures/FiniteElasticStack.env $reference_root/FiniteElasticStack-krakenc/FiniteElasticStack' \
  'FiniteElasticTopN kraken crates/kraken/tests/fixtures/FiniteElasticTopN.env $reference_root/FiniteElasticTopN-kraken/FiniteElasticTopN' \
  'FiniteElasticShearOnly kraken crates/kraken/tests/fixtures/FiniteElasticShearOnly.env $reference_root/FiniteElasticShearOnly-kraken/FiniteElasticShearOnly' \
  'FiniteElasticPower kraken crates/kraken/tests/fixtures/FiniteElasticPower.env $reference_root/FiniteElasticPower-kraken/FiniteElasticPower' \
  'ElasticHalfTopN kraken crates/kraken/tests/fixtures/ElasticHalfTopN.env $reference_root/ElasticHalfTopN-kraken/ElasticHalfTopN' \
  'ElasticHalfBothS kraken crates/kraken/tests/fixtures/ElasticHalfBothS.env $reference_root/ElasticHalfBothS-kraken/ElasticHalfBothS' \
  'ElasticHalfTopBroadband kraken crates/kraken/tests/fixtures/ElasticHalfTopBroadband.env $reference_root/ElasticHalfTopBroadband-kraken/ElasticHalfTopBroadband' \
  'GradedElasticBottomN kraken crates/kraken/tests/fixtures/GradedElasticBottomN.env $reference_root/GradedElasticBottomN-kraken/GradedElasticBottomN' \
  'GradedElasticTopS krakenc crates/kraken/tests/fixtures/GradedElasticTopS.env $reference_root/GradedElasticTopS-krakenc/GradedElasticTopS' \
  'GradedElasticPower krakenc crates/kraken/tests/fixtures/GradedElasticPower.env $reference_root/GradedElasticPower-krakenc/GradedElasticPower' \
  'GradedElasticBio krakenc crates/kraken/tests/fixtures/GradedElasticBio.env $reference_root/GradedElasticBio-krakenc/GradedElasticBio' \
  'TabRefBrc krakenc $reference_root/TabRefCoef/run1/neggradC_brc.env $reference_root/TabRefCoef/run1/neggradC_brc' \
  'TabRefIrc krakenc $reference_root/TabRefCoef/run1/neggradC_irc.env $reference_root/TabRefCoef/run1/neggradC_irc' \
  'BroadBandMunk kraken $reference_root/cases/BroadBand/MunkK.env $reference_root/BroadBand-MunkK-kraken/MunkK' \
  'BroadBandMunkComplex krakenc $reference_root/cases/BroadBand/MunkK.env $reference_root/BroadBand-MunkK-krakenc/MunkK' \
  'GulfAd kraken $reference_root/profile-cases/GulfAd.env $reference_root/GulfAd-kraken/GulfAd' \
  'GulfCm kraken $reference_root/profile-cases/GulfCm.env $reference_root/GulfCm-kraken/GulfCm'; do
  mapping=${mapping//\$reference_root/$reference_root}
  set -- $mapping
  json="$PWD/$reference_root/json/$1.json"
  target/release/pelagic kraken export "$3" --solver "$2" > "$json"
  KRAKEN_JSON_INPUT="$json" KRAKEN_JSON_ENV="$PWD/$3" KRAKEN_JSON_REFERENCE_ROOT="$PWD/$4" \
    tools/reference/test-one.sh --release -p kraken --test differential_reference \
      json_fields_match_fresh_reference -- --ignored --exact --nocapture
  tools/reference/run-cli-kraken.sh "$json" "${json%.json}-results"
  KRAKEN_HDF5_RESULT="${json%.json}-results/$1.h5" KRAKEN_JSON_INPUT="$json" \
  KRAKEN_JSON_ENV="$PWD/$3" KRAKEN_JSON_REFERENCE_ROOT="$PWD/$4" \
    tools/reference/test-one.sh --release -p kraken --test differential_reference \
      json_fields_match_fresh_reference -- --ignored --exact --nocapture
done

step 'BELLHOP line sources, volume loss, beam shift and Gaussian eigenrays'
for case in Field_ThorpLine Field_BioLine; do
  tools/reference/compare-field.sh "crates/bellhop/tests/fixtures/golden/$case.env"
done
for case in BeamShift SimpleGaussian_eigen; do
  tools/reference/compare-ray.sh "crates/bellhop/tests/fixtures/golden/$case.env"
done

step 'BELLHOP irregular receiver pairing (18 pinned comparisons)'
tools/reference/test-one.sh --release -p bellhop --test irregular_pairing \
  irregular_pairing_matches_pinned_reference -- --ignored --exact --nocapture

step 'Critical boundary/interface rays'
tools/reference/check-critical-rays.sh

step 'Official full-grid coherent field (MunkB_Coh)'
tools/reference/compare-field.sh $reference_root/cases/MunkB_Coh.env

step 'Official 50k-receiver ASCII arrival fan (sbcx_Arr_asc)'
tools/reference/compare-arrival.sh $reference_root/cases/sbcx_Arr_asc.env

step 'Actual BELLHOP CLI native and HDF5 full-grid readback'
cargo build --locked --release -p cli
for case in MunkB_Coh sbcx_Arr_asc; do
  input="$PWD/$reference_root/cases/$case.env"
  output="$PWD/$reference_root/$case-cli"
  target/release/pelagic bellhop run "$input" --format both --output "$output"
  BELLHOP_NATIVE_INPUT="$input" BELLHOP_NATIVE_OUTPUT="$output" \
    tools/reference/test-one.sh --release -p cli --test native_output \
      actual_cli_native_layouts_match_all_hdf5_values -- --ignored --exact --nocapture
done
