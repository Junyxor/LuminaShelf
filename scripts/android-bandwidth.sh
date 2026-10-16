#!/usr/bin/env bash
set -Eeuo pipefail

[[ "${GITHUB_ACTIONS:-}" == true && "$(uname -s)" == Linux ]]
state_dir="$PWD/smoke/host-shaper"
mkdir -p "$state_dir"
device=lumina-ifb

interface_name() {
  local interface
  interface="$(cat "$state_dir/interface")"
  [[ "$interface" =~ ^[a-zA-Z0-9_.:-]+$ && "$interface" != lo ]]
  printf '%s' "$interface"
}

status() {
  if [[ -f "$state_dir/ingress-owned" ]]; then
    sudo tc -s filter show dev "$(interface_name)" parent ffff:
  fi
  if [[ -f "$state_dir/device-owned" ]]; then
    sudo tc -s qdisc show dev "$device"
  fi
}

cleanup() {
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
    # This disposable runner must have no pre-existing ingress policy/device.
    if sudo tc qdisc show dev "$interface" | grep -Eq 'qdisc (ingress|clsact) '; then
      echo 'Refusing to replace an existing ingress policy.' >&2
      exit 1
    fi
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
    sudo tc qdisc add dev "$interface" handle ffff: ingress
    touch "$state_dir/ingress-owned"
    # Limit inbound IPv4 HTTPS, including the emulator's public book responses.
    # Local ADB/accessibility traffic does not cross this interface.
    sudo tc filter add dev "$interface" parent ffff: protocol ip pref 49152 u32 match ip protocol 6 0xff match ip sport 443 0xffff action mirred egress redirect dev "$device"
    status >> smoke/host-shaping.txt
    ;;
  *) exit 1 ;;
esac
