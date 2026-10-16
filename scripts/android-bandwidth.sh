#!/usr/bin/env bash
set -Eeuo pipefail

[[ "${GITHUB_ACTIONS:-}" == true && "$(uname -s)" == Linux ]]
state_dir="$PWD/smoke/host-shaper"
mkdir -p "$state_dir"
device=lumina-ifb
filter_priority=321

interface_name() {
  local interface
  interface="$(cat "$state_dir/interface")"
  [[ "$interface" =~ ^[a-zA-Z0-9_.:-]+$ && "$interface" != lo ]]
  printf '%s' "$interface"
}

status() {
  if [[ -f "$state_dir/filter-owned" ]]; then
    sudo tc -s filter show dev "$(interface_name)" ingress pref "$filter_priority"
  fi
  if [[ -f "$state_dir/device-owned" ]]; then
    sudo tc -s qdisc show dev "$device"
  fi
}

cleanup() {
  if [[ -f "$state_dir/filter-owned" ]]; then
    sudo tc filter del dev "$(interface_name)" ingress protocol ip pref "$filter_priority"
    rm -- "$state_dir/filter-owned"
  fi
  if [[ -f "$state_dir/ingress-owned" ]]; then
    sudo tc qdisc del dev "$(interface_name)" ingress
    rm -- "$state_dir/ingress-owned"
  fi
  if [[ -f "$state_dir/device-owned" ]]; then
    sudo ip link del dev "$device"
    rm -- "$state_dir/device-owned"
  fi
}

case "${1:-}" in
  full)
    status >> smoke/host-shaping.txt || true
    cleanup
    ;;
  status)
    status
    ;;
  256)
    cleanup
    interface="$(ip -4 route show default | awk 'NR == 1 { print $5 }')"
    [[ "$interface" =~ ^[a-zA-Z0-9_.:-]+$ && "$interface" != lo ]]
    qdisc_state="$(sudo tc qdisc show dev "$interface")"
    printf '%s\n' "$qdisc_state" > smoke/host-shaping-before.txt
    existing_ingress=false
    if [[ "$qdisc_state" == *'qdisc ingress '* || "$qdisc_state" == *'qdisc clsact '* ]]; then
      existing_ingress=true
      sudo tc filter show dev "$interface" ingress >> smoke/host-shaping-before.txt
      priority_state="$(sudo tc filter show dev "$interface" ingress pref "$filter_priority")"
      if [[ -n "$priority_state" ]]; then
        echo 'Refusing to replace an existing filter at the test priority.' >&2
        exit 1
      fi
    fi
    # Keep the runner's existing ingress/clsact and all its filters intact.
    if ip link show dev "$device" >/dev/null 2>&1; then
      echo 'Refusing to replace an existing shaping device.' >&2
      exit 1
    fi
    printf '%s\n' "$interface" > "$state_dir/interface"
    trap cleanup ERR
    sudo ip link add "$device" type ifb
    touch "$state_dir/device-owned"
    sudo ip link set dev "$device" up
    sudo tc qdisc add dev "$device" root handle 1: tbf rate 256kbit burst 16kb latency 1000ms
    if [[ "$existing_ingress" != true ]]; then
      sudo tc qdisc add dev "$interface" handle ffff: ingress
      touch "$state_dir/ingress-owned"
    fi
    # Limit inbound IPv4 HTTPS, including the emulator's public book responses.
    # Local ADB/accessibility traffic does not cross this interface.
    sudo tc filter add dev "$interface" ingress protocol ip pref "$filter_priority" u32 match ip protocol 6 0xff match ip sport 443 0xffff action mirred egress redirect dev "$device"
    touch "$state_dir/filter-owned"
    status >> smoke/host-shaping.txt
    ;;
  *) exit 1 ;;
esac
