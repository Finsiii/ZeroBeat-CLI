#!/usr/bin/env bash
set -Eeuo pipefail

export LC_ALL=C

readonly SCRIPT_NAME="${0##*/}"
readonly DEFAULT_DURATION=30
readonly DEFAULT_INTERVAL=1
readonly MAX_DURATION=86400
readonly MAX_INTERVAL=3600
readonly MAX_PID=4194304

duration_raw=$DEFAULT_DURATION
interval_raw=$DEFAULT_INTERVAL
declare -a requested_pids=()

usage() {
    cat <<'EOF'
Usage: scripts/benchmark.sh [options]

Read-only benchmark for one already-running ZeroBeat TUI and its daemon. By
default it selects one owned zerobeat-cli and one owned zerobeatd; with stale
daemons, the selected daemon must be the TUI child. Use --pid exactly twice
to select one explicit PID for each role.

Options:
  --duration SECONDS  Measurement duration (default: 30, maximum: 86400)
  --interval SECONDS  Sampling interval (default: 1, maximum: 3600)
  --pid PID           Explicit PID; provide exactly one zerobeat-cli and one zerobeatd
  -h, --help          Show this help

The benchmark reads process metrics from /proc and host metadata only. It does
not launch, pause, signal, play, download, or modify ZeroBeat state.
EOF
}

die_usage() {
    printf '%s: %s\n' "$SCRIPT_NAME" "$*" >&2
    printf 'Try %s --help.\n' "$SCRIPT_NAME" >&2
    exit 2
}

die_runtime() {
    printf '%s: %s\n' "$SCRIPT_NAME" "$*" >&2
    exit 1
}

parse_number() {
    local label=$1
    local raw=$2
    local maximum=$3
    local value

    case "$raw" in
        ''|*[!0-9]*) die_usage "$label must be a positive integer" ;;
    esac
    [ "${#raw}" -le 9 ] || die_usage "$label is too large"
    value=$((10#$raw))
    (( value > 0 )) || die_usage "$label must be greater than zero"
    (( value <= maximum )) || die_usage "$label must be at most $maximum"
    printf '%s' "$value"
}

parse_pid() {
    local raw=$1
    local value

    case "$raw" in
        ''|*[!0-9]*) die_usage 'PID must be a positive integer' ;;
    esac
    [ "${#raw}" -le 9 ] || die_usage 'PID is too large'
    value=$((10#$raw))
    (( value > 0 && value <= MAX_PID )) || die_usage "PID must be between 1 and $MAX_PID"
    printf '%s' "$value"
}

while (( $# > 0 )); do
    case "$1" in
        --duration)
            (( $# >= 2 )) || die_usage '--duration needs a value'
            duration_raw=$2
            shift 2
            ;;
        --interval)
            (( $# >= 2 )) || die_usage '--interval needs a value'
            interval_raw=$2
            shift 2
            ;;
        --pid)
            (( $# >= 2 )) || die_usage '--pid needs a value'
            requested_pids+=("$(parse_pid "$2")")
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die_usage "unknown option: $1"
            ;;
    esac
done

duration=$(parse_number duration "$duration_raw" "$MAX_DURATION")
interval=$(parse_number interval "$interval_raw" "$MAX_INTERVAL")

current_uid=$(id -u)

process_comm() {
    local pid=$1
    local comm

    [ -r "/proc/$pid/comm" ] || return 1
    comm=$(<"/proc/$pid/comm") || return 1
    case "$comm" in
        zerobeat-cli|zerobeatd) printf '%s' "$comm" ;;
        *) return 1 ;;
    esac
}

process_is_owned() {
    local pid=$1
    local uid

    [ -r "/proc/$pid/status" ] || return 1
    uid=$(awk '$1 == "Uid:" { print $2; exit }' "/proc/$pid/status") || return 1
    [ "$uid" = "$current_uid" ]
}

process_parent_pid() {
    local pid=$1
    local parent_pid

    [ -r "/proc/$pid/status" ] || return 1
    parent_pid=$(awk '$1 == "PPid:" { print $2; exit }' "/proc/$pid/status") || return 1
    [[ "$parent_pid" =~ ^[0-9]+$ ]] || return 1
    printf '%s' "$parent_pid"
}

validate_process() {
    local pid=$1
    local comm

    comm=$(process_comm "$pid") ||
        die_runtime "PID $pid disappeared or comm is not exactly zerobeat-cli/zerobeatd"
    process_is_owned "$pid" ||
        die_runtime "PID $pid is not owned by the current UID ($current_uid)"
    printf '%s\t%s' "$pid" "$comm"
}

discover_processes() {
    local proc pid comm parent_pid

    for proc in /proc/[0-9]*; do
        [ -d "$proc" ] || continue
        pid=${proc##*/}
        comm=$(process_comm "$pid") || continue
        process_is_owned "$pid" || continue
        parent_pid=$(process_parent_pid "$pid") || continue
        printf '%s\t%s\t%s\n' "$pid" "$comm" "$parent_pid"
    done
}

declare -a pids=()
declare -a names=()
if (( ${#requested_pids[@]} > 0 )); then
    (( ${#requested_pids[@]} == 2 )) ||
        die_usage 'explicit --pid mode requires exactly two PIDs: one zerobeat-cli and one zerobeatd'
    declare -A seen_pids=()
    for pid in "${requested_pids[@]}"; do
        [ -z "${seen_pids[$pid]+x}" ] || die_usage "PID $pid was provided more than once"
        seen_pids["$pid"]=1
        process_info=$(validate_process "$pid")
        IFS=$'\t' read -r validated_pid validated_name <<< "$process_info"
        pids+=("$validated_pid")
        names+=("$validated_name")
    done
    zerobeat_cli_count=0
    zerobeatd_count=0
    for name in "${names[@]}"; do
        case "$name" in
            zerobeat-cli) zerobeat_cli_count=$((zerobeat_cli_count + 1)) ;;
            zerobeatd) zerobeatd_count=$((zerobeatd_count + 1)) ;;
        esac
    done
    (( zerobeat_cli_count == 1 && zerobeatd_count == 1 )) ||
        die_usage 'explicit --pid mode requires exactly one zerobeat-cli and one zerobeatd PID'
else
    mapfile -t discovered < <(discover_processes | sort -n -k1,1)
    declare -a tui_candidates=()
    declare -a daemon_candidates=()
    declare -a daemon_parents=()
    for process_info in "${discovered[@]}"; do
        [ -n "$process_info" ] || continue
        IFS=$'\t' read -r discovered_pid discovered_name discovered_parent <<< "$process_info"
        case "$discovered_name" in
            zerobeat-cli) tui_candidates+=("$discovered_pid") ;;
            zerobeatd)
                daemon_candidates+=("$discovered_pid")
                daemon_parents+=("$discovered_parent")
                ;;
        esac
    done
    if (( ${#tui_candidates[@]} != 1 )); then
        die_runtime "expected exactly one owned zerobeat-cli TUI, found ${#tui_candidates[@]}; pass --pid TUI_PID --pid DAEMON_PID"
    fi
    (( ${#daemon_candidates[@]} > 0 )) ||
        die_runtime "no owned zerobeatd daemon found; pass --pid TUI_PID --pid DAEMON_PID"
    tui_pid=${tui_candidates[0]}
    declare -a child_daemons=()
    for index in "${!daemon_candidates[@]}"; do
        if [ "${daemon_parents[$index]}" = "$tui_pid" ]; then
            child_daemons+=("${daemon_candidates[$index]}")
        fi
    done
    (( ${#child_daemons[@]} == 1 )) ||
        die_runtime "found ${#daemon_candidates[@]} owned zerobeatd processes but could not identify exactly one child of zerobeat-cli TUI PID $tui_pid; pass --pid TUI_PID --pid DAEMON_PID"
    daemon_pid=${child_daemons[0]}
    pids=("$tui_pid" "$daemon_pid")
    names=(zerobeat-cli zerobeatd)
fi

(( ${#pids[@]} == 2 )) || die_runtime 'benchmark requires exactly one zerobeat-cli TUI and one zerobeatd daemon'

read_process_stat() {
    local pid=$1
    local fields utime stime starttime

    [ -r "/proc/$pid/stat" ] || return 1
    fields=$(awk '{ sub(/^.*\) /, ""); print $12, $13, $20 }' "/proc/$pid/stat") || return 1
    read -r utime stime starttime <<< "$fields"
    [[ "$utime" =~ ^[0-9]+$ && "$stime" =~ ^[0-9]+$ && "$starttime" =~ ^[0-9]+$ ]] || return 1
    printf '%s %s' "$((utime + stime))" "$starttime"
}

read_uptime_seconds() {
    local uptime

    [ -r /proc/uptime ] || return 1
    uptime=$(awk '{ print $1; exit }' /proc/uptime) || return 1
    [[ "$uptime" =~ ^[0-9]+([.][0-9]+)?$ ]] || return 1
    printf '%s' "$uptime"
}

read_rss_kib() {
    local pid=$1
    local rss

    [ -r "/proc/$pid/status" ] || return 1
    rss=$(awk '$1 == "VmRSS:" { print $2; found=1; exit } END { if (!found) exit 1 }' "/proc/$pid/status") || return 1
    [[ "$rss" =~ ^[0-9]+$ ]] || return 1
    printf '%s' "$rss"
}

declare -A previous_ticks=()
declare -A process_starttime=()
declare -A current_ticks=()
declare -A current_rss_kib=()

collect_snapshot() {
    local pid stat_fields ticks starttime rss

    for pid in "${pids[@]}"; do
        process_comm "$pid" >/dev/null ||
            die_runtime "PID $pid disappeared or changed comm during measurement"
        process_is_owned "$pid" ||
            die_runtime "PID $pid is no longer owned by the current UID"
        stat_fields=$(read_process_stat "$pid") ||
            die_runtime "could not read /proc/$pid/stat; the process may have disappeared"
        read -r ticks starttime <<< "$stat_fields"
        [ "$starttime" = "${process_starttime[$pid]}" ] ||
            die_runtime "PID $pid was reused during measurement (starttime changed)"
        rss=$(read_rss_kib "$pid") ||
            die_runtime "could not read /proc/$pid/status; the process may have disappeared"
        current_ticks["$pid"]=$ticks
        current_rss_kib["$pid"]=$rss
    done
}

benchmark_start_uptime=''
for pid in "${pids[@]}"; do
    stat_fields=$(read_process_stat "$pid") ||
        die_runtime "could not read /proc/$pid/stat before sampling"
    read -r previous_pid_ticks pid_starttime <<< "$stat_fields"
    previous_ticks["$pid"]=$previous_pid_ticks
    process_starttime["$pid"]=$pid_starttime
done
benchmark_start_uptime=$(read_uptime_seconds) || die_runtime 'could not read monotonic time from /proc/uptime'
interval_start_uptime=$benchmark_start_uptime

clk_tck=$(getconf CLK_TCK)
[[ "$clk_tck" =~ ^[0-9]+$ && "$clk_tck" -gt 0 ]] || die_runtime 'could not determine CLK_TCK'

total_cpu_ticks=0
total_rss_kib=0
peak_rss_kib=0
peak_interval_cpu=0
samples=0
remaining=$duration

while (( remaining > 0 )); do
    sleep_for=$(( remaining < interval ? remaining : interval ))
    sleep "$sleep_for"
    collect_snapshot
    interval_end_uptime=$(read_uptime_seconds) || die_runtime 'could not read monotonic time from /proc/uptime'
    interval_seconds=$(awk -v start="$interval_start_uptime" -v end="$interval_end_uptime" 'BEGIN { delta = end - start; if (delta <= 0) exit 1; printf "%.9f", delta }') ||
        die_runtime 'monotonic clock did not advance during a sample'

    interval_ticks=0
    combined_rss_kib=0
    for pid in "${pids[@]}"; do
        delta_ticks=$(( current_ticks["$pid"] - previous_ticks["$pid"] ))
        (( delta_ticks >= 0 )) || die_runtime "CPU counter moved backwards for PID $pid"
        interval_ticks=$(( interval_ticks + delta_ticks ))
        total_cpu_ticks=$(( total_cpu_ticks + delta_ticks ))
        previous_ticks["$pid"]=${current_ticks["$pid"]}
        combined_rss_kib=$(( combined_rss_kib + current_rss_kib["$pid"] ))
    done

    total_rss_kib=$(( total_rss_kib + combined_rss_kib ))
    (( combined_rss_kib > peak_rss_kib )) && peak_rss_kib=$combined_rss_kib
    interval_cpu=$(awk -v ticks="$interval_ticks" -v hz="$clk_tck" -v seconds="$interval_seconds" 'BEGIN { printf "%.3f", ticks * 100 / hz / seconds }')
    interval_cpu_num=$(awk -v value="$interval_cpu" 'BEGIN { print value + 0 }')
    awk -v value="$interval_cpu_num" -v peak="$peak_interval_cpu" 'BEGIN { exit !(value > peak) }' && peak_interval_cpu=$interval_cpu_num
    samples=$(( samples + 1 ))
    remaining=$(( remaining - sleep_for ))
    interval_start_uptime=$interval_end_uptime
done
benchmark_end_uptime=$interval_end_uptime
actual_duration=$(awk -v start="$benchmark_start_uptime" -v end="$benchmark_end_uptime" 'BEGIN { delta = end - start; if (delta <= 0) exit 1; printf "%.3f", delta }') ||
    die_runtime 'monotonic clock did not advance during the benchmark'

distro='unknown'
if [ -r /etc/os-release ]; then
    detected_distro=$(awk -F= '$1 == "PRETTY_NAME" { gsub(/^"|"$/, "", $2); print $2; exit }' /etc/os-release)
    [ -n "$detected_distro" ] && distro=$detected_distro
fi
kernel=$(uname -sr)
cpu_model=$(awk -F: '/^model name[[:space:]]*:/ { gsub(/^[[:space:]]+/, "", $2); print $2; exit }' /proc/cpuinfo)
if [ -z "$cpu_model" ]; then
    cpu_model=$(awk -F: '/^(Hardware|Processor)[[:space:]]*:/ { gsub(/^[[:space:]]+/, "", $2); print $2; exit }' /proc/cpuinfo)
fi
[ -n "$cpu_model" ] || cpu_model='unknown'
online_cpus=$(getconf _NPROCESSORS_ONLN 2>/dev/null || printf '%s' 'unknown')

pid_report=''
for index in "${!pids[@]}"; do
    pid_report+="${pid_report:+, }${names[$index]}=${pids[$index]}"
done

mean_rss_mib=$(awk -v total="$total_rss_kib" -v count="$samples" 'BEGIN { printf "%.1f", total / count / 1024 }')
peak_rss_mib=$(awk -v peak="$peak_rss_kib" 'BEGIN { printf "%.1f", peak / 1024 }')
mean_cpu=$(awk -v ticks="$total_cpu_ticks" -v hz="$clk_tck" -v seconds="$actual_duration" 'BEGIN { printf "%.1f", ticks * 100 / hz / seconds }')
peak_cpu=$(awk -v value="$peak_interval_cpu" 'BEGIN { printf "%.1f", value }')

printf 'ZeroBeat benchmark (read-only /proc)\n'
printf 'Distro: %s\n' "$distro"
printf 'Kernel: %s\n' "$kernel"
printf 'CPU: %s (%s online)\n' "$cpu_model" "$online_cpus"
printf 'PIDs: %s\n' "$pid_report"
printf 'Requested duration: %ss | Actual duration: %ss | Requested interval: %ss | Samples: %s\n' "$duration" "$actual_duration" "$interval" "$samples"
printf 'Combined mean RSS: %s MiB\n' "$mean_rss_mib"
printf 'Combined peak RSS: %s MiB\n' "$peak_rss_mib"
printf 'Combined overall mean CPU: %s%% (100%% = one core)\n' "$mean_cpu"
printf 'Combined peak interval CPU: %s%%\n' "$peak_cpu"
