# Pico 2 W Rust voice assistant

> [!CAUTION]
> i gave up the project because the Raspberry Pi Pico 2 W is too weak. now i am working on an esp32-s3 soution https://github.com/trainvent/ESPal

This repository currently implements the **audio hardware diagnostic milestone**.
It is not yet a networked voice assistant: Wi-Fi, audio transport, OpenAI sessions,
manual conversation activation, timeout and playback echo suppression remain to be implemented.
No hardware test has passed yet. The verified checks are compilation and host tests only.

## Hardware

Raspberry Pi Pico 2 W (RP2350, Arm Cortex-M33), ICS-43434 microphone,
and MAX98357A speaker amplifier. Wiring is assumed from the handoff and must be checked.

| Pico connection | Microphone | Amplifier |
| --- | --- | --- |
| GP10 | BCLK | BCLK |
| GP11 | LRCL / WS | LRC |
| GP12 | DOUT | — |
| GP13 | — | DIN |
| 3V3 | 3V | — |
| VBUS / 5V | — | VIN |
| GND | GND and SEL (left) | GND |

Check the amplifier module's SD/MODE and gain configuration against its documentation.
Both transmitted slots contain the same mono signal, allowing left, right or mixed
channel selection. GPIO25 is not used as an onboard LED on this wireless board.

Timing references: [ICS-43434 datasheet](https://product.tdk.com/system/files/dam/doc/product/sw_piezo/mic/mems-mic/data_sheet/ds-000069-ics-43434-v1.2.pdf)
and [MAX98357A datasheet](https://www.analog.com/media/en/technical-documentation/data-sheets/MAX98357A-MAX98357B.pdf).

## Build and tests

The firmware supports `thumbv8m.main-none-eabihf`. The old `.pico-rs` selector
and RP2040/RISC-V linker scripts are legacy starter files; they do not select this firmware's target.
The build script copies the RP2350 memory layout into Cargo's output directory
and no longer rewrites `.cargo/config.toml` during builds.

```sh
rustup target add thumbv8m.main-none-eabihf
cargo build --locked --release
cargo test --locked --lib --target x86_64-unknown-linux-gnu
cargo clippy --locked --release --all-features -- -D warnings
```

The default firmware captures continuously and transmits **silence**. For a quiet
1 kHz triangle wave lasting one second every five seconds:

```sh
cargo build --locked --release --features test-tone
```

The tone peak is approximately 1.6% of digital full scale. Acoustic loudness
still depends on the amplifier gain and speaker. There is no microphone-to-speaker
loopback. Building either variant replaces the ELF at:
`target/thumbv8m.main-none-eabihf/release/pi_ai_rpipico`.
Always rebuild the intended variant before flashing.

## Flash and observe

With an SWD probe connected to SWDIO, SWCLK and ground:

```sh
probe-rs run --chip RP235x target/thumbv8m.main-none-eabihf/release/pi_ai_rpipico
```

This flashes the ELF and displays defmt RTT statistics. A USB connection directly
to the Pico is not an SWD probe. This firmware does not expose USB serial.
RTT logging is explicitly nonblocking so a disconnected debugger cannot stop audio.

Alternatively, use Raspberry Pi's [picotool](https://github.com/raspberrypi/picotool)
with the Pico attached in BOOTSEL mode. `picotool` is not installed in the current environment.
With a current picotool installation:

```sh
picotool uf2 convert target/thumbv8m.main-none-eabihf/release/pi_ai_rpipico audio.uf2 --family rp2350-arm-s
```

Hold BOOTSEL while connecting USB, then copy `audio.uf2` onto the Pico's mounted
bootloader drive. Bootloader USB supports flashing; an SWD probe is still required
for the current capture diagnostics. Do not interpret a successful flash as an audio test.

## Hardware acceptance checks

1. Build and flash the default silent variant. Check wiring and power first.
2. If a logic analyzer is available, measure GP10 at approximately 3.072 MHz,
   GP11 at approximately 48 kHz, with 64 BCLK cycles per stereo frame,
   32 cycles per slot and LRCLK changing one bit before the next MSB.
   The fractional PIO divider gives a small sample-rate approximation error.
3. With RTT, speak or tap near the microphone. After startup the firmware reports
   `peak`, `rms`, `clipped`, `samples` and `stall_windows` once per 48,000 captured samples.
   Peak/RMS are PCM16 amplitudes, not dB or calibrated sound pressure.
   The first 100 ms are ignored. Clipped should normally be zero. Quiet-room
   readings should be lower than nearby speech. Zero or fixed readings need investigation.
4. Flash the `test-tone` build. Confirm a periodic quiet tone, then restore the
   default build. Capture continues during playback, so statistics may include
   acoustic speaker pickup. Echo suppression is not implemented at this milestone.
5. Run for several minutes. After the startup interval, `stall_windows` must stay
   zero. This is the number of 10 ms checks observing sticky TX/RX stall flags,
   not the number of lost samples. A nonzero value invalidates timing acceptance.
6. Stats do not establish recognizable speech. Export/listen to captured audio
   in a subsequent USB or network transport milestone before claiming that check passed.

## Implementation and next work

One PIO0 state machine drives both clock pins, outputs GP13 and samples GP12.
It shifts MSB-first, sampling on BCLK rising edges and updating output on falling
edges. LRCLK changes during the previous slot's last bit for the I2S one-bit delay.
Four PIO cycles per bit produce 256 cycles per stereo frame. Each FIFO word is a
32-bit slot; the microphone's top 16 bits become signed PCM16. Low padding bits
are discarded. Left microphone slots alone contribute to diagnostics.

Four DMA channels form two ping-pong transfers, with two 10 ms buffers in each
direction (15,360 bytes total). The CPU processes/requeues buffers while the other
buffers stream. This is a bounded audio pipeline, but its real-time behavior still
requires device testing. Long CPU stalls can pause the shared clocks; sticky PIO
stall flags report that failure. Host tests execute the assembled PIO instructions
to check clock spacing, channel transitions and the wrap boundary, and test PCM
conversion, statistics and tone amplitude/period.

Next milestones:

- Add captured PCM export and verify recognizable speech on the physical device.
- Add CYW43 Wi-Fi and bounded streaming transport while preserving uninterrupted DMA service.
- Choose and implement the service path (direct TLS/WebSocket or a backend relay).
  Keep API keys out of version control and logs; a relay can keep them off the Pico.
- Match the [Pi 5 reference](https://github.com/trainvent/pi-ai): manually activated
  conversation, configurable 1–600 second duration (60 by default), concise replies
  in the user's language, and microphone silence while speaker audio is queued
  plus at least 0.5 seconds afterward. This is turn-taking, not acoustic echo cancellation.
- Implement proper filtered conversion from 48 kHz microphone audio to the service's
  required sample rate after checking current API documentation.

Verification on 2026-10-05: default and tone release builds passed; four host tests
passed; Clippy passed with warnings denied. No Pico or SWD debug probe was discovered,
so flashing, audible playback, microphone quality and uninterrupted hardware timing
remain unverified.
