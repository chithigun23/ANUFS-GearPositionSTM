# Power and gear-output notes

Design notes for the board's power input and the gear-voltage output. Reflects the netlist as of 2026-11-11.

## Sources

- **Car:** the 5 V sensor supply from the MoTeC M150 (`SEN_5V`, J6 pin 1). Not a battery rail, so no 12 V, reverse-battery or load-dump handling.
- **USB-C:** VBUS from J5 (`V_USB`), for bench programming and debug.

## Architecture

```
M150 sensor 5 V (J6.1) --1 uF--> U6 TPS2116 VIN1
                                  |  VOUT (MUX_OUT) --> F1 (200 mA PTC) --> /5V --> U1 AP2112K-3.3 --> +3V3
USB-C VBUS (V_USB) ----1 uF--> U6 VIN2          |
                                                +--> U5 TLV9062 V+ (100 nF)
```

- **U6 `TPS2116`** is in priority mode (MODE tied to VIN1). VIN1 (car) is used while PR1 is above V_REF, otherwise VIN2 (USB).
- It blocks reverse current, so the M150 supply is never back-fed from USB and a laptop is never back-fed from the car (datasheet leakage is about 1 to 2 uA on the unused input).
- Rated 1.6 to 5.5 V in, absolute max 6 V. Fine for a clean 5 V sensor supply.

## Switchover threshold

`Vth = V_REF x (1 + R13 / R14)`, with V_REF = 1.00 V typical (0.92 to 1.08 V).

- `R13` = 300 k (VIN1 to PR1), `R14` = 100 k (PR1 to GND)
- Vth = 4.0 V typical, 3.68 to 4.32 V over V_REF tolerance
- Divider draws 12.5 uA at 5 V
- Chosen so the switch happens well above the LDO's roughly 3.55 V minimum input, and well below the car supply's normal 5 V

## Gear-voltage output

STM32 DAC (`PA4`) into a `TLV9062` op-amp, with a divider readback on an ADC pin.

```
PA4 --+-- U5 +in      U5 out --+-- GEAR_OUT (J6.3)
      |                 |      |
     R10 100k          R11 4.99k  R9 10k
      |                 |      |
     GND         -in ---+     GEAR_OUT_DIV --> PA1
                  |                |
                 R12 10k          R15 15k
                  |                |
                 GND              GND
```

- **Gain:** `1 + R11 / R12 = 1.499`. A 4.1 V output needs about 2.74 V from the DAC, which is code about 3394 at a 3.3 V VDDA.
- **DAC buffer off.** With the buffer on the output cannot go below 0.2 V, so 0 V ("fault / unknown") would be unreachable. Buffer-off output impedance is up to 15 kohm, fine into an op-amp input.
- **R10 pulldown** keeps the op-amp input at 0 V while `PA4` floats during reset, so `GEAR_OUT` is 0 V until the firmware enables the DAC.
- **DAC specs** (STM32F072 datasheet, table 60): INL +-4 LSB, offset +-10 mV, gain error +-0.5%, settling 4 us max, VDDA 2.4 to 3.6 V. The output is ratiometric to VDDA.
- **Readback:** ratio 0.6, so 4.1 V reads as about 2.46 V, and the op-amp's rail limit of about 5 V reads as about 3.0 V. Source impedance is 6 kohm, so use an ADC sample time of 13.5 cycles or longer (datasheet RAIN max is 11.4 kohm at 13.5 cycles).
- **Closed loop:** trims out DAC offset and gain error, op-amp offset and resistor error. It cannot remove VDDA error by itself, because the DAC and ADC both scale with VDDA. Use `VREFINT` (ADC channel 17) with the factory `VREFINT_CAL` value at `0x1FFFF7BA` to compute the real VDDA and convert readings to volts. Use 0.1% resistors in the divider.
- **Timing:** set the output from a lookup table at the start of a shift and trim in the background while the gear is steady. Do not wait on the ADC inside the shift path (the old build budgets about 15 to 20 ms from sensor to output). Use a deadband of a couple of DAC counts so the trim does not hunt.
- **Fault detection:** a readback far from the commanded value (for example 0 V when a gear was commanded) means a stuck or shorted output.

## CAN

`U4 TCAN337` runs from 3.3 V (recommended 3.0 to 3.6 V). Its bus pins are ISO 11898-2 compatible, have +-14 V fault protection and a +-12 V common-mode range, and go high-impedance when the board is unpowered. The datasheet states it is compatible with 5 V CAN transceivers on the same bus. There is no 120 ohm termination on the board, so this assumes the board is mid-bus or the bus is terminated elsewhere.

## Load

All logic runs from 3.3 V through the LDO, so the 5 V input current equals the 3.3 V load current, plus the op-amp and readback divider on the 5 V side.

| Load | Typical | Worst case |
|---|---|---|
| TCAN337 | about 3.5 mA idle | 55 to 60 mA dominant |
| STM32F072 at 48 MHz | about 25 mA | about 30 mA |
| DAC, op-amp, readback divider, LDO quiescent | about 1 to 3 mA (estimate) | about 3 mA (estimate) |
| **Total** | **about 40 to 60 mA** | **about 90 to 100 mA** |

`F1` (200 mA hold) sits on the mux output. A PTC derates when hot, so this is tight at the top end in a warm car. Its resistance is around an ohm, which is irrelevant for the LDO input.

## Decisions

- **Mux, not diodes.** Diodes would also have worked. The power saving is nil, because the LDO current is the same either way. The mux was kept for the guaranteed car priority and the lack of a diode drop.
- **No buck converter.** 12 V is not used, the supply is already 5 V.
- **No reverse-polarity stage or TVS on `SEN_5V`.** The M150 generates its sensor 5 V internally, so reversing the battery does not put a negative voltage on `SEN_5V`. Confirm the M150's own reverse-battery behavior in its manual.
- **STM32 DAC plus op-amp, not an MCP4801.**
  - The MCP4801's output cannot exceed its 3.3 V supply, and on 5 V its SPI inputs would need level shifting (inputs need at least 0.7 x VDD).
  - The STM32 DAC is 12-bit with better datasheet offset and gain error, removes a chip, and the TLV9062 is already used on APPPD.

## Open items

- Tie the unused op-amp channel as a follower: `U5` pin 6 to pin 7, and pin 5 to GND (currently pin 5 is on `+3V3`, pin 6 on GND, pin 7 open).
- Check whether the duplicate test points `TP14` (+3V3), `TP15` (CAN+) and `TP16` (CAN-) are intentional.
- `ST` (pin 8 of `U6`) is unconnected. `PA2` is free if you want to read which source is active (pull up to 3.3 V, high = car, low = USB).
- Check the M150 sensor 5 V output current rating against the board's roughly 100 mA worst case.
- Confirm the TLV9062 is rated for a 5 V supply.
- Consider more bulk capacitance on `MUX_OUT` (only 100 nF there, 10 uF and 1 uF are behind `F1`).
- The neutral switch input (J6 pin 6) goes straight to `PB2` with no series resistor or clamp.
