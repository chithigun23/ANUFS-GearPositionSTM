# GearSensor STM32

[![GearSensor STM32 architecture diagram](docs/architecture.svg)](docs/architecture.svg)

Gear position sensor board for a MoTeC M150 paddle-shift system: an `STM32F072` reads a linear position sensor and the neutral switch, works out the current gear, and reports it to the ECU as an analog voltage, with a CAN transceiver on the ECU bus. Powered from the M150 sensor 5 V or USB-C, and replaces the Arduino UNO R4 WiFi and shield build described on the [GearSensor project page](https://chithigunatilake.com/GearSensor/).

## Architecture

- **Power** - 5 V in from the M150 sensor supply (`SEN_5V`, J6) or from USB-C VBUS. A `TPS2116` power mux gives the car priority and falls back to USB when the car rail drops below about 4 V (set by `R13`/`R14`, 300 k / 100 k). It blocks reverse current, so the ECU supply is never back-fed from a laptop and the laptop is never back-fed from the car. 1 uF on each mux input, a 200 mA polyfuse (`F1`) on the mux output and an `AP2112K-3.3` LDO after it. There is no reverse-polarity stage because a sensor supply cannot be reversed by the battery.
- **Compute** - `STM32F072CB` (LQFP-48) with a 24 MHz `ABM8AIG` crystal on HSE, which CAN needs for accurate bit timing. Ferrite-filtered VDDA, NRST and BOOT0 buttons.
- **Position and neutral sensing** - linear position sensor on J6 pin 7, scaled by `R7`/`R8` (10 k / 15 k) so a 5 V sensor swing lands inside the 3.3 V ADC range on `PA0`. Neutral switch on J6 pin 6 into `PB2`.
- **Gear output** - the STM32's own 12-bit DAC on `PA4` (output buffer off, so it can reach 0 V) drives a `TLV9062` op-amp at a gain of 1.5 (`R11` 4.99 k / `R12` 10 k) from the 5 V rail, giving the gear-position voltage on J6 pin 3 up to about 4.1 V. `R10` (100 k) holds the output at 0 V until the firmware enables the DAC. A 10 k / 15 k divider (`R9`/`R15`) reads the output back on `PA1` for closed-loop trim.
- **CAN** - `TCAN337` 3.3 V transceiver on `PB9` / `PB8`, with fault on `PB0` and silent mode on `PB1`. CAN+ / CAN- on J6 pins 5 and 4.
- **Debug and USB** - `J5` USB-C with 5.1 k CC pull-downs and a `USBLC6-2SC6` ESD array on D+ / D- into the MCU's native USB (`PA11` / `PA12`). 4-pin SWD header `J1`. Test pads on the rails and every external signal.

The diagram is animated: dashes show the direction of flow, amber for power, teal for signals, blue for buses. If your viewer does not animate SVGs, the arrows still show the same paths.

## Car connector (J6)

| Pin | Net | Function |
|---|---|---|
| 1 | `SEN_5V` | M150 sensor 5 V in |
| 2 | `GND` | Ground |
| 3 | `GEAR_OUT` | Gear voltage out to the ECU |
| 4 | `CAN-` | CAN low |
| 5 | `CAN+` | CAN high |
| 6 | `NEUT_SW` | Neutral switch |
| 7 | `SPS` | Linear position sensor signal |
| 8 | - | Not connected |

## From the Arduino version

- Same job, same sensors, same ECU interface. The shift logic, trust model and FMEA are documented on the [project page](https://chithigunatilake.com/GearSensor/).
- One board replaces the Arduino UNO R4 WiFi and shield.
- The gear-voltage output is read back through an ADC pin again, which version 2 of the Arduino build dropped.
- This board has no WiFi, Bluetooth or LED matrix, so the phone display and bench LED build do not carry over. USB-C is the way in for programming and debug.
- This repo is hardware only. There is no firmware in it yet.

## Status

Work in progress. The schematic is being finished and the PCB is not yet routed. Not run on a car.

## Repo contents

- [KiCad project](gearPosSens.kicad_pro)
- [Schematic](gearPosSens.kicad_sch)
- [PCB layout](gearPosSens.kicad_pcb)
- [Symbols, footprints and 3D models](library)
