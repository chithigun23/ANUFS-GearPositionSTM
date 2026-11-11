# Power, gear-output and LED notes

Design notes for the board's power input, the gear-voltage output and the status LEDs. Reflects the netlist as of 2026-11-11 (03:16 export).

## Sources

- **Car:** the 5 V sensor supply from the MoTeC M150 (`SEN_5V`, J3 pin 1). Not a battery rail, so no 12 V, reverse-battery or load-dump handling. The previous Arduino build already ran from this supply.
- **USB-C:** VBUS from J1 (`V_USB`), for bench programming and debug.

## Architecture

```
M150 sensor 5 V (J3.1) --C4 1 uF--> U2 TPS2116 VIN1
                                     |  VOUT (MUX_OUT) --> F1 (300 mA PTC) --> +5V --> U1 AP2112K-3.3 --> +3V3
USB-C VBUS (V_USB) ----C5 1 uF--> U2 VIN2          |
                                                   +--> U4 TLV9062 V+ (C15 100 nF)
                                                   +--> SW3 slide switch --> LED_5V --> 6 LEDs
```

- **U2 `TPS2116`** is in priority mode (MODE tied to VIN1). VIN1 (car) is used while PR1 is above V_REF, otherwise VIN2 (USB).
- It blocks reverse current, so the M150 supply is never back-fed from USB and a laptop is never back-fed from the car (datasheet leakage is about 1 to 2 uA on the unused input).
- Rated 1.6 to 5.5 V in, absolute max 6 V. Fine for a clean 5 V sensor supply.

## Switchover threshold

`Vth = V_REF x (1 + R3 / R4)`, with V_REF = 1.00 V typical (0.92 to 1.08 V).

- `R3` = 300 k (VIN1 to PR1), `R4` = 100 k (PR1 to GND)
- Vth = 4.0 V typical, 3.68 to 4.32 V over V_REF tolerance
- Divider draws 12.5 uA at 5 V
- The M150 sensor supply does not dip, so the exact threshold matters little.

## Gear-voltage output

STM32 DAC (`PA4`) into a `TLV9062` op-amp (`U4`), with a divider readback on an ADC pin.

```
PA4 --+-- U4 +in      U4 out --+-- R29||R30 (390 ohm each) --+-- GEAR_OUT (J3.3), D3 TVS
      |                        |                              |
     R10 1M                R11 4.99k                      R14 10k
      |                        |                              |
     GND               -in ----+                       GEAR_OUT_DIV --> PA1
                         |                                    |
                        R12 10k                           R15 15k
                         |                                    |
                        GND                                  GND
```

- **Gain:** `1 + R11 / R12 = 1.499`. A 4.1 V output needs about 2.74 V from the DAC, which is code about 3394 at a 3.3 V VDDA.
- **DAC buffer off.** With the buffer on the output cannot go below 0.2 V, so 0 V ("fault / unknown") would be unreachable. Buffer-off output impedance is up to 15 kohm, fine into an op-amp input.
- **R10 pulldown** (1 M) keeps the op-amp input at 0 V while `PA4` floats during reset, so `GEAR_OUT` is 0 V until the firmware enables the DAC. It is 1 M, not lower, because the buffer-off DAC output impedance is up to 15 k and ST wants at least 1.5 M of load for 1% accuracy. With 1 M the loss is at most about 1.5% (a 100 k pulldown would have lost up to about 13%).
- **Series resistors R29/R30:** about 195 ohm total protects the op-amp output from the harness. The readback divider sits after them, so the closed loop measures what the ECU sees and absorbs the drop across them.
- **Unused channel:** `U4` pins 6 and 7 are tied together (follower) and pin 5 is on GND.
- **DAC specs** (STM32F072 datasheet, table 60): INL +-4 LSB, offset +-10 mV, gain error +-0.5%, settling 4 us max, VDDA 2.4 to 3.6 V. The output is ratiometric to VDDA.
- **TLV9062 specs** (datasheet): 1.8 to 5.5 V supply, rail-to-rail input and output, offset +-1.6 mV max at 5 V, short-circuit current +-50 mA.
- **Readback:** ratio 0.6, so 4.1 V reads as about 2.46 V, and the op-amp's rail limit of about 5 V reads as about 3.0 V. Source impedance is 6 kohm, so use an ADC sample time of 13.5 cycles or longer (datasheet RAIN max is 11.4 kohm at 13.5 cycles).
- **Closed loop:** trims out DAC offset and gain error, op-amp offset and resistor error. It cannot remove VDDA error by itself, because the DAC and ADC both scale with VDDA. Use `VREFINT` (ADC channel 17) with the factory `VREFINT_CAL` value at `0x1FFFF7BA` to compute the real VDDA and convert readings to volts. Use 0.1% resistors in the divider.
- **Timing:** set the output from a lookup table at the start of a shift and trim in the background while the gear is steady. Do not wait on the ADC inside the shift path (the old build budgets about 15 to 20 ms from sensor to output). Use a deadband of a couple of DAC counts so the trim does not hunt.
- **Fault detection:** a readback far from the commanded value (for example 0 V when a gear was commanded) means a stuck or shorted output.

## Harness inputs

- **TVS:** one `AQ1005-01ETG` (bidirectional, 6 V standoff, about 9 V clamp at 1 A, AEC-Q101) on each of `SPS` (`D1`), `NEUT_SW` (`D2`) and `GEAR_OUT` (`D3`), at the J3 pad, other end to GND. Figures are from DigiKey and search excerpts because the Littelfuse PDF returned 403, so confirm against the real datasheet.
- **Series resistors:** `R5` (10 k) before `PA0`, `R28` (1 k) before `PB2`.
- **CAN:** no separate TVS. The TCAN337 has on-chip +-12 kV IEC 61000-4-2 contact ESD and +-14 V bus fault protection. A CAN-specific TVS is optional margin if the harness runs near the actuator solenoids. Do not use the AQ1005 on CAN (6 V standoff is below the +-12 V common-mode range).

## Status LEDs

Six `XL-2012SURUGC` LEDs. Per the datasheet each holds two independent dies (red: pin 1 cathode, pin 2 anode; green: pin 3 cathode, pin 4 anode), not a common-anode part.

- **Rail:** anodes (pins 2 and 4) of all six LEDs share `LED_5V`. `SW3` (`CJS-1200TB1`, SPDT, 100 mA at 6 V DC) connects its common (pin 2) to `+5V` and one throw (pin 1) to `LED_5V`, so opening it turns every LED off.
- **Why 5 V:** the green die is 2.6 to 3.4 V at 20 mA, too high for the 3.3 V rail.
- **Drive:** each cathode goes through its own resistor to a 5 V-tolerant MCU pin set open-drain. Low turns the die on, high-Z turns it off, and pins are high-Z during reset so the LEDs stay dark.
- **Resistors:** 390 ohm red and 680 ohm green. That is about 7 mA red and 3 mA green, which targets half the luminous intensity telemetry runs its LEDs at (telemetry drives about 10 mA per die). These are estimates because the datasheet gives forward voltage only at 20 mA, so trim on the bench.
- **Brightness balance:** typical intensity at 20 mA is 150 mcd red and 1000 mcd green, so green stays much brighter than red. Raise the green resistors (about 1.5 k) to balance them.

| LED | Role | Red die | Green die |
|---|---|---|---|
| `LED1` | gear code | `PB10` | `PB11` |
| `LED2` | gear code | `PB12` | `PB13` |
| `LED3` | gear code | `PB14` | `PB15` |
| `LED4` | gear code | `PA8` | `PA15` |
| `LED5` | MCU status | `PB3` | `PB4` |
| `LED6` | neutral switch | `PB5` | `PB6` |

## Mux status

`U2` pin 8 (`ST`, open-drain) goes to `PA2` through a global label `MUX_STAT`. High means the car (VIN1) is powering the board, low means USB (VIN2), both inputs off, or thermal shutdown. The datasheet says `ST` should be pulled up, so firmware enables the internal pull-up on `PA2` (add an external 10 k to 3.3 V if you want a stronger one). The capacitance on the other side of `F1` is treated as enough for the mux output.

## CAN

`U5 TCAN337` runs from 3.3 V (recommended 3.0 to 3.6 V). Its bus pins are ISO 11898-2 compatible, have +-14 V fault protection and a +-12 V common-mode range, and go high-impedance when the board is unpowered. The datasheet states it is compatible with 5 V CAN transceivers on the same bus. There is no 120 ohm termination on the board, so this assumes the board is mid-bus or the bus is terminated elsewhere.

## Crystal

`Y1 ABM8AIG-24.000MHz-8-2Z-T` is an 8 pF load crystal (ESR 50 ohm max for 20 to 26 MHz, drive level 100 uW max). Two 8 pF load capacitors give a load of 4 pF plus stray, which is 7 to 9 pF for a typical 3 to 5 pF stray, close to 8 pF. ST suggests 10 pF of stray as a rough estimate, which would make it about 14 pF. That is a small frequency error and well within CAN's tolerance. No series resistor is needed.

## Load

All logic runs from 3.3 V through the LDO, so the 5 V input current equals the 3.3 V load current, plus the op-amp, readback divider and LEDs on the 5 V side.

| Load | Typical | Worst case |
|---|---|---|
| TCAN337 | about 3.5 mA idle | 55 to 60 mA dominant |
| STM32F072 at 48 MHz | about 25 mA | about 30 mA |
| DAC, op-amp, readback divider, LDO quiescent | about 1 to 3 mA (estimate) | about 3 mA (estimate) |
| LEDs (about 7 mA red, 3 mA green per die) | about 30 mA with one die per LED | about 60 mA with every die lit |
| **Total** | **about 70 to 90 mA** | **about 150 to 160 mA** |

`F1` (300 mA hold) sits on the mux output. A PTC derates when hot, and its resistance is around an ohm, which is irrelevant for the LDO input. Firmware should avoid lighting every die at once.

## Footprints

Policy: 0402 where possible, otherwise 0805, avoid 0603.

- All new resistors (`R11`, `R16` to `R30`) are 0402, and the ferrite bead `FB1` is 0402.
- `F1` is a 0805 fuse footprint, since a 300 mA PTC in 0402 is unlikely to exist.
- `J3` uses the Molex `43045-0800` (Micro-Fit 3.0, 2x4) footprint copied from APPPD, which also uses it on its harness connector.
- Library parts are in `library/`, with symbols for `TLV9062xD`, `XL-2012SURUGC`, `CJS-1200TB1`, `AQ1005-01ETG`, `TS-1088-AR02016` and `TC2030` copied from APPPD, the BSPD and the telemetry board. `J4` uses the Tag-Connect `TC2030-IDC-FP` footprint.

## Decisions

- **Mux, not diodes.** Diodes would also have worked. The power saving is nil, because the LDO current is the same either way. The mux was kept for the guaranteed car priority and the lack of a diode drop.
- **No buck converter.** 12 V is not used, the supply is already 5 V.
- **No reverse-polarity stage or TVS on `SEN_5V`.** The M150 generates its sensor 5 V internally, so reversing the battery does not put a negative voltage on `SEN_5V`. Confirm the M150's own reverse-battery behavior in its manual.
- **STM32 DAC plus op-amp, not an MCP4801.**
  - The MCP4801's output cannot exceed its 3.3 V supply, and on 5 V its SPI inputs would need level shifting (inputs need at least 0.7 x VDD).
  - The STM32 DAC is 12-bit with better datasheet offset and gain error, removes a chip, and the TLV9062 is already used on APPPD.

## Open items

- **ERC:** clean (0 errors, 0 warnings from kicad-cli). `PWR_FLAG`s sit on `GND`, `+5V`, VDDA, `SEN_5V` and `V_USB`, the duplicate `5V` label and the dangling `GEAR_OUT` label are removed, and `SW1`/`SW2` use a project-library symbol (copied from APPPD) with passive pins.
- **Debug:** `J2` is a 4-pin SWD header with no reset line. `J4` is a Tag-Connect TC2030 footprint (SWDIO, SWCLK, NRST, 3.3 V, GND; pin 6 SWO unused, since the F0 has no trace) copied from the telemetry board.
- Check the M150 sensor 5 V output current rating against the board's roughly 150 mA worst case.
- Confirm the AQ1005-01ETG numbers against the real Littelfuse datasheet.
