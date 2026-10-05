#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

drive="${PICO_BOOT_DRIVE:-/media/${USER}/RP2350}"
if [[ ! -f "$drive/INFO_UF2.TXT" ]] || ! grep -q 'Board-ID: RP2350' "$drive/INFO_UF2.TXT"; then
    echo "Connect the Pico while holding BOOTSEL, release it, then run again."
    echo "Expected boot drive: $drive (override with PICO_BOOT_DRIVE)."
    exit 1
fi

picotool="${PICOTOOL_PATH:-}"
if [[ -z "$picotool" ]]; then
    picotool="$(command -v picotool || true)"
fi
if [[ -z "$picotool" ]]; then
    shopt -s nullglob
    tools=("$HOME"/.pico-sdk/picotool/*/picotool/picotool)
    if (( ${#tools[@]} )); then
        picotool="${tools[${#tools[@]}-1]}"
    fi
fi
if [[ ! -x "$picotool" ]]; then
    echo "picotool not found. Set PICOTOOL_PATH to the picotool executable."
    exit 1
fi

cargo build --locked --release
elf=target/thumbv8m.main-none-eabihf/release/pi_ai_rpipico
uf2=target/thumbv8m.main-none-eabihf/release/pi_ai_rpipico.uf2
"$picotool" uf2 convert "$elf" -t elf "$uf2" --family rp2350-arm-s
cp "$uf2" "$drive/"
echo "Firmware copied. The Pico will reboot and run it automatically."
echo "For the next upload, reconnect while holding BOOTSEL."
