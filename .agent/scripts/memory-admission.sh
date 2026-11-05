#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 Julian Y. Richard Corbet
# SPDX-License-Identifier: FSL-1.1-ALv2
# Memory admission for build workers: print how many build jobs fit.
#
#   source memory-admission.sh; jobs="$(ccvl_memory_admission)"
#
# Inputs: CARGO_BUILD_JOBS (default 2), CI_MIN_AVAILABLE_MB (default 8192) and
# CI_MEMORY_PER_JOB_MB (default 2048), all positive integers. The kernel
# interfaces are read from CCVL_PROC_ROOT (default /proc) and CCVL_CGROUP_ROOT
# (default /sys/fs/cgroup) so fixtures can supply them. The decision goes to
# stderr; only the job count goes to stdout. Refusals exit non zero.

ccvl_positive_setting() {
  local name="$1" default="$2" value
  value="${!name:-$default}"
  if [[ ! $value =~ ^[0-9]+$ ]] || ((10#$value <= 0)); then
    printf '%s must be a positive integer\n' "$name" >&2
    return 1
  fi
  printf '%s\n' "$((10#$value))"
}

ccvl_memory_admission() {
  local proc_root="${CCVL_PROC_ROOT:-/proc}" mount="${CCVL_CGROUP_ROOT:-/sys/fs/cgroup}"
  local jobs reserve per_job available kilobytes line group_path group limit current
  local pressure avg10 cpus fit
  jobs="$(ccvl_positive_setting CARGO_BUILD_JOBS 2)" || return 1
  reserve="$(ccvl_positive_setting CI_MIN_AVAILABLE_MB 8192)" || return 1
  per_job="$(ccvl_positive_setting CI_MEMORY_PER_JOB_MB 2048)" || return 1
  kilobytes=
  while IFS= read -r line; do
    if [[ $line == MemAvailable:* ]]; then
      read -r _ kilobytes _ <<<"$line"
      break
    fi
  done <"$proc_root/meminfo"
  [[ $kilobytes =~ ^[0-9]+$ ]] || { echo 'MemAvailable is unavailable' >&2; return 1; }
  available=$((kilobytes / 1024))
  group_path=
  if [[ -r $proc_root/self/cgroup ]]; then
    while IFS= read -r line; do
      if [[ $line == 0::* ]]; then group_path="${line:3}"; break; fi
    done <"$proc_root/self/cgroup"
  fi
  group="$mount/${group_path#/}"
  [[ -e $group/memory.max ]] || group="$mount"
  if [[ -e $group/memory.max ]]; then
    limit="$(<"$group/memory.max")"
    if [[ $limit != max ]]; then
      current="$(<"$group/memory.current")"
      fit=$(((limit - current) / 1048576))
      ((fit >= 0)) || fit=0
      ((fit >= available)) || available=$fit
    fi
  fi
  pressure="$proc_root/pressure/memory"
  if [[ -e $pressure ]]; then
    avg10=0
    while IFS= read -r line; do
      if [[ $line == full\ * ]]; then
        for field in $line; do
          if [[ $field == avg10=* ]]; then avg10="${field#avg10=}"; fi
        done
        break
      fi
    done <"$pressure"
    if awk -v value="$avg10" 'BEGIN { exit !(value >= 5) }'; then
      echo 'Memory admission refused: sustained full memory pressure' >&2
      return 1
    fi
  fi
  cpus="$(nproc 2>/dev/null || echo 1)"
  ((jobs <= cpus)) || jobs=$cpus
  fit=$(((available - reserve) / per_job))
  ((available > reserve)) || fit=0
  ((jobs <= fit)) || jobs=$fit
  if ((jobs == 0)); then
    printf 'Memory admission refused: %s MiB available with %s MiB reserved\n' "$available" "$reserve" >&2
    return 1
  fi
  printf 'Memory admission: %s MiB available, %s MiB reserve, %s build jobs\n' "$available" "$reserve" "$jobs" >&2
  printf '%s\n' "$jobs"
}
