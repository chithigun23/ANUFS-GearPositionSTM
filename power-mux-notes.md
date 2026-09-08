# Power Mux (Car 12V + USB-C 5V) — Session Notes

Summary of the design/implementation session for adding a safe power mux to `gearPosSens.kicad_sch`. Written mid-session (schematic placement not yet complete — see **Status** at the bottom).

## The ask

Board needs to run from either the car's 12V harness or a USB-C connection (bench programming/debug), safely combined so neither source backfeeds into the other.

## What the board already had (before this session)

- **J5** — 16-pin USB-C receptacle. VBUS runs through **F1** (a 100mA polyfuse) into a net called `VIN_LDO`, which feeds **U1** (`AP2112K-3.3`, SOT-23-5 LDO, 3.8–6V in) → `+3.3V` for the STM32.
- **U3** (`USBLC6-2SC6`) already protects the D+/D- lines.
- **J4** is a `telemetry:2601-1102` WAGO 2-pole terminal block — already fully used for CAN+/CAN-, not power.
- **U4** (`TCAN337` CAN transceiver) runs on **3.3V**, not 5V — confirmed by tracing a `power:+3.3V` symbol at (186.69, 110.49) wired straight down to it. (Earlier in the session I'd guessed it might be on the raw 5V rail — that was wrong.)
- A `VIN_LDO`-net test point (TP8) exists, and there's a literal TODO left in the schematic (as a text annotation):
  > `Todo:\nTC2030 SWD\nPower mux for USB Power and Car 12V`
- **R2** and **R3** (both 5.1kΩ, 0805) are already wired to J5's CC1 and CC2 pins — **these are already-correct USB-C sink pulldowns**. I initially and incorrectly told the user CC1/CC2 were floating (I'd only grepped for text labels named "CC1"/"CC2" and missed that they connect via a direct routed wire to R2/R3 instead of a net label). This was caught by precisely reverse-engineering J5's pin geometry and cross-checking it against real wires in the file — **no new CC pulldown resistors are needed.**

## Proposed architecture

```
CAR 12V ──[J6 WAGO 2-pole]──┬── TVS1 (SMBJ33A) ── GND
                             │
                        F2 (PTC ~0.5A)
                             │
                        D1 (Schottky, reverse-polarity block)
                             │
                     U6 Buck 12V→5V (LMR33630ADDAR)
                             │
                          "5V_CAR" ──┐
                                     ├── U7 Power Mux (TPS2113APWR, ideal-diode) ── "5V_SYS" (relabeled VIN_LDO)
USB-C J5 VBUS ── F1 (existing) ── "5V_USB" ──┘                                          │
                                                                    ┌─────────────────────┼─────────────────┐
                                                              U1 AP2112K-3.3         U4 TCAN337          sensor
                                                              → 3V3 (MCU)          (already 3.3V,        excitation
                                                                                    unaffected)             tap
```

Key idea: the mux's `OUT` pin gets relabeled back to `VIN_LDO` — the exact net name already used everywhere downstream (U1, sensor excitation tap, etc.) — so nothing already wired to that net needs to be touched. Only the segment between F1 and U1 gets relabeled from `VIN_LDO` to `5V_USB` to feed the mux's IN2 instead.

## Component selections (final)

All sourced via `easyeda2kicad --full --lcsc_id=...` (pulls symbol+footprint+3D from the real LCSC/EasyEDA part data) rather than hand-drawn, specifically so pin assignments come from source data, not memory.

| Ref | Part | Package | LCSC | Notes |
|---|---|---|---|---|
| J6 | WAGO 2601-1102 (reuse) | THT | — | Same part as J4, no new library entry needed |
| TVS1 | **SMBJ33A** | SMB/DO-214AA | C499719 | 33V standoff, clamps automotive load-dump above 14.4V charging |
| F2 | Generic PTC (stock `Device:Polyfuse`) | 0603 | — | ~0.5A hold, same footprint style as existing F1 |
| D1 | **PMEG6010ER** | SOD-123W | C456121 | Series Schottky, reverse-polarity protection ahead of the buck |
| U6 | **LMR33630ADDAR** | HTSSOP-8 PowerPAD | C841384 | 12V→5V buck. Pinout: 1=PGND, 2=VIN, 3=EN, 4=PG, 5=FB, 6=VCC, 7=BOOT, 8=SW, 9=EP(=PGND) |
| U7 | **TPS2113APWR** | TSSOP-8 | C130052 | Ideal-diode power mux. Pinout: 1=STAT, 2=~EN, 3=VSNS, 4=ILIM, 5=GND, 6=IN2, 7=OUT, 8=IN1 |

**Correction made mid-session:** I originally told the user TPS2113A came in an MSOP-10 (`TPS2113ADGNR`) based on misreading a PDF pinout caption. The actual stocked/orderable LCSC part is **TPS2113APWR**, an 8-pin **TSSOP** — a different, easier-to-hand-solder package. The library entry uses the corrected part.

**Deliberately no new library part for:** R9/R10 (not needed at all — see R2/R3 finding above), F2 (stock `Device:Polyfuse`), J6 (reuses existing `telemetry:2601-1102`).

### Key design values (from datasheet/search verification, not guessed)

- **LMR33630ADDAR feedback divider:** Vref = 0.8V, TI recommends top resistor (VOUT→FB) = 100kΩ. For 5.0V out: bottom resistor (FB→GND) = 100k / 5.25 ≈ **19.1kΩ** (standard E96 value) → Vout ≈ 4.99V.
  - EN tied to VIN (always-on, no MCU control needed).
  - BOOT cap: 100nF (standard across the whole TI bootstrap-buck family).
  - Planned L1 ≈ 22µH, Cin ≈ 10µF/50V, Cout ≈ 22µF/16V (conservative, safe values for a <0.5A load; not WEBENCH-optimized).
  - Exposed pad (pin 9) ties to the same PGND as pin 1.
- **TPS2113APWR:**
  - ILIM: `I_LIMIT(mA) = 500 / R_ILIM(kΩ)`. Planned R_ILIM ≈ 500Ω → ~1A limit (headroom over an assumed <300–400mA board load).
  - VSNS: pulling it to GND puts the part in **autoswitching mode** — it automatically picks whichever of IN1/IN2 is higher, with ideal-diode reverse-current blocking. (No separate "priority pin" exists on this part — I'd implied one earlier in the conversation; that was imprecise. Pulling VSNS high instead would force fixed IN1-priority, but autoswitching is simpler and was the actual goal, so GND is the plan.)
  - EN (`~EN`, active-low): tie to GND to permanently enable.
  - STAT: left unconnected (valid per datasheet); could add a status LED later if wanted, not part of this pass.

## Library work completed this session

Added to `GearPositionSTM/library/` (nickname `gear`), each with `.kicad_sym` + `.kicad_mod` + `.step`:
- `gear:SMBJ33A`, `gear:PMEG6010ER`, `gear:LMR33630ADDAR`, `gear:TPS2113APWR`

All four verified with `kicad-cli sym export svg` / `fp export svg` (both resolve and render). One hiccup along the way: renaming `PMEG6010ER`'s symbol left its internal child-unit symbol name mismatched (`PMEG6010ER,115_0_1` instead of `PMEG6010ER_0_1`), which made `kicad-cli sym upgrade` fail with a bare "Unable to load library" — fixed by renaming the child unit to match.

Saved as a memory ([project_gear_library.md](../../../Users/chith/.claude/projects/C--Claude/memory/project_gear_library.md) in Claude's memory store) so this convention doesn't need to be re-derived next session.

## Git state

- Repo: `C:\Users\chith\Desktop\GearPositionSTM` (branch `main`, up to date with `origin/main`).
- **Not yet committed.** `git status` shows pre-existing uncommitted changes that predate this session (modified `.kicad_pcb`/`.kicad_prl`/`.kicad_pro`, an untracked `readme.md`, and a `.history` submodule with new commits — none of these were touched by me) plus this session's untracked additions (the four new library parts under `library/`).
- User asked to commit before/after the schematic placement specifically so a bad placement can be reverted. **This has not happened yet** — no commits made this session.

## Status — what's NOT done yet

The actual schematic placement (adding J6, TVS1, F2, D1, U6, U7 + support passives to `gearPosSens.kicad_sch`, wiring it up, relabeling the `VIN_LDO` net, resizing the page to A3 for room, and committing to git) was **in progress and got interrupted before any schematic edits were made**. Everything above (library parts, values, architecture) is decided and ready to place; no lines in `gearPosSens.kicad_sch` have been changed yet.

Last thing in progress: confirming R2/R3's far ends actually land on a GND symbol (found a plausible candidate — `#PWR014` at (201.93, 50.8) — but hadn't fully traced the wire chain to confirm before being interrupted).

**Next step (pending your go-ahead):** resume placement — commit the current working-tree state as a baseline checkpoint, add the new parts/wiring to the schematic, validate with `kicad-cli sch erc` and a rendered preview, then commit that as its own change so it's a clean single revert if the layout doesn't work for you.
