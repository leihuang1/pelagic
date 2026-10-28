#!/usr/bin/env bash
# A parameter sweep as a Slurm job array. Each array task solves its share of
# the cases with `run-many`, and a task that runs again skips what it finished.
#
#   find "$PWD/cases" -name '*.json' | sort > cases.txt
#   sbatch examples/cli/slurm-sweep.sh kraken cases.txt results
#   sbatch --array=7 examples/cli/slurm-sweep.sh kraken cases.txt results   # task 7 again
#   python3 examples/cli/sweep-status.py results cases.txt
#
# The list, the cases and the results folder must be on storage that every
# node can reach. Uses `pelagic` on PATH, or the program named by $PELAGIC_CLI.
# To try it without Slurm, run the tasks yourself:
#
#   for i in 0 1 2; do
#       SWEEP_SHARDS=3 SLURM_ARRAY_TASK_ID=$i examples/cli/slurm-sweep.sh kraken cases.txt results
#   done
#
#SBATCH --job-name=pelagic-sweep
#SBATCH --array=0-49
#SBATCH --cpus-per-task=4
#SBATCH --mem-per-cpu=1G
#SBATCH --time=01:00:00
#SBATCH --requeue
set -euo pipefail
usage="usage: slurm-sweep.sh bellhop|kraken LIST OUTPUT"
model=${1:?$usage}
list=${2:?$usage}
out=${3:?$usage}
pelagic=${PELAGIC_CLI:-pelagic}
# The number of tasks in --array above. It is set here rather than read from
# Slurm, so that a task resubmitted on its own still takes the same share.
shards=${SWEEP_SHARDS:-50}

# Task I of N solves cases I, I+N, I+2N, ... of the sorted list, and appends a
# line per case to OUTPUT/status-I-of-N.ndjson. pelagic uses the CPUs that
# Slurm binds the task to; if `nproc` inside a job prints the whole node's
# count, add --workers "$SLURM_CPUS_PER_TASK" --threads 1.
"$pelagic" "$model" run-many --list "$list" --output "$out" \
    --shard "$SLURM_ARRAY_TASK_ID/$shards" --resume
