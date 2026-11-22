# Gear position sensor firmware

Rust firmware for the STM32F072CB gear position sensor board, written to the [firmware plan](../docs/firmware-plan.md).

| Crate | What it is |
|---|---|
| [`gps-core`](gps-core) | All the logic, with no hardware types: calibration math, gear order, debouncing, the shift state machine, closed-loop output control and the LED decoding. `no_std`, no `unsafe`, tested on the PC. |
| [`gps-fw`](gps-fw) | The board binary (Embassy): clocks, ADC scan, DAC with its buffer off, LEDs, watchdog and the 5 ms loop. It makes no decisions. |

## Build, test, flash

From this folder:

```powershell
cargo core-test
```

```powershell
cargo fw
```

```powershell
cargo fw-run
```

`cargo core-test` runs the `gps-core` tests on the PC. `cargo fw` builds the release binary for `thumbv6m-none-eabi`. `cargo fw-run` flashes it with `probe-rs` and streams the `defmt` log; it needs a probe on `J2` (SWD) or the TC2030 (`J4`). The aliases are in [`.cargo/config.toml`](.cargo/config.toml).

## Calibrating

Measure `R5`, `R6`, `R11`, `R12`, `R14`, `R15`, `R29` and `R30` with a meter. Enter the values in `NOMINAL` in [`gps-core/src/config.rs`](gps-core/src/config.rs), along with `vdda_trim_ppm` (meter VDDA divided by the VDDA in the log, in parts per million), and reflash. The thresholds, timings and gear voltages are in the same struct.

Before connecting the car, measure the neutral wire with the switch open. If it sits above 3.6 V, set `NEUTRAL_PULLUP` to `false` in [`gps-fw/src/main.rs`](gps-fw/src/main.rs).

## LEDs

See [Status LEDs](../readme.md#status-leds) in the main readme. The decoding is in [`gps-core/src/led.rs`](gps-core/src/led.rs).

## Not verified on hardware yet

- The firmware has not run on a board. Follow the bring-up list in the firmware plan.
- The F0 ADC clock: the HAL does not start HSI14, so `main.rs` starts it before the ADC calibrates.
- `assume_first_when_leaving_neutral` defaults to `false`: the neutral switch opening at rest with no shift is reason 3 (output 0 V). Set it to `true` to assume gear 1 instead. Check this against what the Arduino build's flag of the same name did.
