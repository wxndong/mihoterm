# Shared CPU limits for developer and release validation scripts.
#
# Keep local validation responsive on shared machines. `nproc` reports the
# processors available to this process, including affinity/cpuset restrictions
# on supported systems. Maintainers can opt in to a different limit with
# MIHOTERM_BUILD_JOBS.

if command -v nproc >/dev/null 2>&1; then
  mihoterm_available_cpus="$(nproc)"
else
  mihoterm_available_cpus="$(getconf _NPROCESSORS_ONLN 2>/dev/null || printf '1')"
fi
if [[ ! "$mihoterm_available_cpus" =~ ^[1-9][0-9]*$ ]]; then
  mihoterm_available_cpus=1
fi

mihoterm_default_jobs=$((mihoterm_available_cpus / 4))
if (( mihoterm_default_jobs < 1 )); then
  mihoterm_default_jobs=1
fi

mihoterm_build_jobs="${MIHOTERM_BUILD_JOBS:-$mihoterm_default_jobs}"
if [[ ! "$mihoterm_build_jobs" =~ ^[1-9][0-9]*$ ]] \
  || (( mihoterm_build_jobs > mihoterm_available_cpus )); then
  echo "MIHOTERM_BUILD_JOBS must be between 1 and the available CPU count ($mihoterm_available_cpus)" >&2
  exit 2
fi

export CARGO_BUILD_JOBS="$mihoterm_build_jobs"
export RAYON_NUM_THREADS="$mihoterm_build_jobs"
export RUST_TEST_THREADS="$mihoterm_build_jobs"
