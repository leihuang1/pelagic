# Sweeps on a cluster

A sweep of many independent cases runs with
[`pelagic … run-many`](cli.md#many-cases). On one machine, a single command
uses every CPU. On a cluster, a job array gives each task a share of the cases
with `--shard`, and `--resume` lets a task that runs again skip what it already
finished. Results are bit-for-bit the same however the cases are split.

## One machine

```sh
find "$PWD/cases" -name '*.json' | sort > cases.txt
pelagic kraken run-many --list cases.txt --output results --resume
```

This starts one worker per case, up to one per available CPU, so there is no
need to start several `pelagic` processes yourself. Each case goes to
`results/<stem>/`, and `results/status.ndjson` gets one line per case. Use
absolute paths in the list, or always start from the same folder: `--resume`
matches a case by its path as written.

## Slurm job arrays

[`examples/cli/slurm-sweep.sh`](../../examples/cli/slurm-sweep.sh) is a
complete job script:

```sh
find "$PWD/cases" -name '*.json' | sort > cases.txt
sbatch examples/cli/slurm-sweep.sh kraken cases.txt results
```

Its core is:

```sh
#SBATCH --array=0-49
#SBATCH --cpus-per-task=4
pelagic kraken run-many --list cases.txt --output results \
    --shard "$SLURM_ARRAY_TASK_ID/50" --resume
```

- **Shares.** Task `I` of `N` solves cases `I`, `I+N`, `I+2N`, … of the sorted
  list, so neighbouring cases, which often cost about the same, go to different
  tasks. Every task checks the whole list for clashing names first, so all of
  them accept or reject the same list.
- **Number the array from 0**, and write `N` into the script instead of
  reading `SLURM_ARRAY_TASK_COUNT`: a task resubmitted on its own with
  `--array=7` would see a count of 1.
- **Status files.** Each task appends to its own
  `results/status-I-of-N.ndjson` and writes only its own cases' folders, so
  tasks never touch each other's files. Every node needs the list, the cases
  and `results/` on shared storage.
- **CPUs.** Slurm binds each task to its `--cpus-per-task` CPUs. On Linux,
  `pelagic` uses only the CPUs it is bound to, and no more than a cgroup CPU
  quota allows, so `--cpus-per-task` alone sets the split between workers and
  threads. To check the binding, run `srun --cpus-per-task=4 nproc`: it should
  print 4. If it prints the node's full count, your cluster does not bind
  CPUs; add `--workers "$SLURM_CPUS_PER_TASK" --threads 1`.
- **Exit codes.** A task's exit code is the largest of its cases' codes, so a
  task with a failed case shows as failed in `sacct` while its other cases
  still finish.

## PBS

PBS Pro:

```sh
#!/bin/bash
#PBS -N pelagic-sweep
#PBS -J 0-49
#PBS -l select=1:ncpus=4:mem=4gb
#PBS -l walltime=01:00:00
cd "$PBS_O_WORKDIR"
pelagic kraken run-many --list cases.txt --output results \
    --shard "$PBS_ARRAY_INDEX/50" --resume
```

Torque uses `#PBS -t 0-49`, `#PBS -l nodes=1:ppn=4` and `$PBS_ARRAYID`
instead. Whether a PBS job is bound to its CPUs depends on the site; check
with `nproc` as above, and if it is not bound, add
`--workers "$NCPUS" --threads 1` (`$PBS_NUM_PPN` on Torque).

## Memory

Each worker holds one case at a time, so a task needs about the number of
workers times the peak of one case. Measure one typical case with the number
of threads it will get:

```sh
/usr/bin/time -v pelagic kraken run case.json --threads 1 --output /tmp/one   # Linux
/usr/bin/time -l pelagic kraken run case.json --threads 1 --output /tmp/one   # macOS
```

and read "Maximum resident set size". With one worker per CPU, ask for that
much per CPU (`--mem-per-cpu`) plus some margin. If that is more than the nodes
have, use fewer workers with more threads each, for example
`--workers 2 --threads 2` on 4 CPUs. BELLHOP's ray records grow with threads,
by up to 32 MiB per thread. KRAKEN's mode-shape scratch also grows with threads:
up to 64 MiB in all, then about 60 bytes per mesh row per thread. On eight or more
threads, KRAKEN frequencies and profiles that run side by side can also hold about
128 MiB of finished results.

## When a task stops early

Every successful case writes its status line after all file syncs and
installation. Each publication, including a separate `run` in a job array,
syncs its files concurrently with at most 16 I/O participants including the
calling thread. This is independent of the numerical `--threads` setting;
a refused helper leaves work to the caller rather than failing the case.
Linux and Windows workers sync their own files without cross-case coordination
or a filesystem-wide flush. Each file keeps its own error result. Apple can
share its additional drive-cache flush between ready cases.

Each helper thread holds one open file, so helpers are shared by the whole
process: at most 64, and at most a quarter of the open files left after 8 for
each worker and 32 for the process. When the limit leaves no such room, each
worker syncs its files one after another, as before, and a helper that finds
no free descriptor leaves its file to its worker, which tries it once more
alone. The reserve is an estimate of what the CLI's workers need, not a
guarantee. At startup the CLI raises its soft open-file limit
(`ulimit -Sn`) to the hard limit (`ulimit -Hn`), as Go and Node.js programs do
(on macOS, to at most 10240). If the hard limit is also Linux's usual 1024,
sweeps of up to 92 workers keep all 64 helpers and 124 or more get none.
No additional directory sync or directory-entry power-loss guarantee is added,
and the status file is not fsynced per line. Node failure can lose a record
or result names, so resume rehashes every listed artifact rather than trusting
the record alone.
Submit the task again:

```sh
sbatch --array=7 examples/cli/slurm-sweep.sh kraken cases.txt results
```

With `--resume`, it skips each recorded case whose input, options and results
are unchanged, and solves the rest. A case that failed runs again. Retained
scratch, locks or backups are never adopted: interrupted publication may need
manual recovery first. See the [publication crash states](native-output.md#run-many-barriers-and-interrupted-batches)
before removing control paths or backups. Storage-server durability still
depends on the cluster filesystem honoring sync requests. The example
script also asks Slurm to requeue the task by itself after a node failure or
preemption (`#SBATCH --requeue`). `--resume` reads every status file in
`results/`, so a case done by any task, or by an earlier one-machine run, is
skipped.

Keep the list and `N` the same while a sweep is under way. Adding a case
changes which task gets which case. `--resume` still skips every case that is
done, but a case's lines are then spread over several status files, and
`sweep-status.py` may report it as unclear (see below).

Never run two copies of the same task at once: both would write the same
folders and status file.

## Progress and failures

[`examples/cli/sweep-status.py`](../../examples/cli/sweep-status.py) reads all
the status files together, takes each case's last line in each file, and
lists failures and cases that have no line yet. It needs only Python 3.6 or
newer:

```console
$ python3 examples/cli/sweep-status.py results cases.txt
1000 cases recorded: 997 done, 3 failed; 12 of 1012 listed cases have no record
failed  /home/me/cases/c0042.json: KR0202 unsupported JSON schema version 1; expected 2 (exit code 2)
...
missing /home/me/cases/c0101.json
```

It exits with 1 while any case is failed, unclear or missing. A case is
**unclear** when it is done in one status file and failed in another. Lines
carry no time, so the reporter cannot tell which is newer. This happens when
an earlier one-machine run, or runs with a different `N`, used the same
results folder. Once every task of the newer run has finished, move the older
run's status files out of `results/`. The newer files then have a line for
every case, which is all `--resume` needs.

The status file format is described in the
[command-line guide](cli.md#status-file-and-resuming).

## Many processes on one machine

`run-many` covers most uses. Separate processes are still useful when cases
need different options, such as a different `.flp` file for each KRAKEN case.
Give each process one thread so that they do not compete for the CPUs:

```sh
export PELAGIC_THREADS=1
# xargs
xargs -P 8 -I{} sh -c 'pelagic kraken run "$1" --output "results/$(basename "$1" .json)"' sh {} < cases.txt
# GNU parallel
parallel -j 8 pelagic kraken run {} --output results/{/.} :::: cases.txt
```

Memory is then the number of processes times the peak of one case. These runs
write no status file and cannot resume. A rerun replaces the earlier results.
