//! Status LED decoding (readme "Status LEDs", firmware plan "LEDs").
//!
//! Six red/green LEDs. `LED1` to `LED3` are the code LEDs with weights 1, 2 and 4, `LED4` is
//! MODE, `LED5` is STAT and `LED6` is NEUT.

use crate::fsm::{Reason, State};
use crate::gear::Gear;

pub const MODE: usize = 3;
pub const STAT: usize = 4;
pub const NEUT: usize = 5;

/// Which dies to light. Index 0 is `LED1`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct LedFrame {
    pub red: [bool; 6],
    pub green: [bool; 6],
}

impl LedFrame {
    /// Show a 3-bit value on the code LEDs in one colour.
    fn code(&mut self, value: u8, red: bool) {
        for bit in 0..3 {
            if value & (1 << bit) != 0 {
                if red {
                    self.red[bit] = true;
                } else {
                    self.green[bit] = true;
                }
            }
        }
    }

    /// Number of dies lit (each sinks current from the MCU).
    pub fn dies_on(&self) -> usize {
        self.red
            .iter()
            .chain(self.green.iter())
            .filter(|&&d| d)
            .count()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LedInputs {
    pub state: State,
    /// Debounced neutral switch.
    pub neutral: bool,
    /// Running on USB power (`MUX_STAT` low).
    pub on_usb: bool,
    /// Time since boot, for blinking.
    pub time_ms: u32,
}

/// The LED pattern for the current state.
pub fn frame(inp: &LedInputs) -> LedFrame {
    let mut f = LedFrame::default();
    match inp.state {
        State::Armed(g) => {
            f.code(g.led_code(), false);
            f.green[MODE] = true;
        }
        State::WaitRelease { dest, .. } => {
            f.code(dest.led_code(), false);
            f.red[MODE] = true;
            f.green[MODE] = true;
        }
        State::Unknown(r) => {
            f.code(r as u8, true);
            f.red[MODE] = (inp.time_ms / 250).is_multiple_of(2); // 2 Hz blink
        }
        State::Fault(r) => {
            f.code(r as u8, true);
            f.red[MODE] = true;
        }
    }

    // STAT: 1 Hz heartbeat from the main loop, so it stops if the loop stalls.
    if (inp.time_ms / 500).is_multiple_of(2) {
        if matches!(inp.state, State::Fault(_)) {
            f.red[STAT] = true;
        } else if inp.on_usb {
            f.red[STAT] = true;
            f.green[STAT] = true;
        } else {
            f.green[STAT] = true;
        }
    }

    // NEUT: red while the count and the switch disagree, otherwise green when closed.
    if inp.state == State::Unknown(Reason::Disagree) {
        f.red[NEUT] = true;
    } else if inp.neutral {
        f.green[NEUT] = true;
    }
    f
}

/// Lamp test at boot: one LED (both dies) at a time, left to right on the board
/// (NEUT, STAT, MODE, 4, 2, 1), 700 ms in total. `None` when finished.
pub fn lamp_test(time_ms: u32) -> Option<LedFrame> {
    const STEP_MS: u32 = 117; // 700 / 6, rounded up so the six steps fit in 700 ms
    let i = (time_ms / STEP_MS) as usize;
    if time_ms >= 700 || i >= 6 {
        return None;
    }
    let led = 5 - i; // LED6 first
    let mut f = LedFrame::default();
    f.red[led] = true;
    f.green[led] = true;
    Some(f)
}

/// Gear shown on the code LEDs, decoded back (for tests and logging).
pub fn shown_code(f: &LedFrame) -> (u8, u8) {
    let bits = |d: &[bool; 6]| (0..3).fold(0u8, |v, b| v | (u8::from(d[b]) << b));
    (bits(&f.green), bits(&f.red))
}

/// Convenience: the gear a code means, if it is a gear code.
pub fn gear_from_code(code: u8) -> Option<Gear> {
    match code {
        7 => Some(Gear::N),
        1 => Some(Gear::G1),
        2 => Some(Gear::G2),
        3 => Some(Gear::G3),
        4 => Some(Gear::G4),
        5 => Some(Gear::G5),
        6 => Some(Gear::G6),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(state: State, t: u32) -> LedFrame {
        frame(&LedInputs {
            state,
            neutral: false,
            on_usb: false,
            time_ms: t,
        })
    }

    #[test]
    fn gears_in_green_binary() {
        for g in [
            Gear::N,
            Gear::G1,
            Gear::G2,
            Gear::G3,
            Gear::G4,
            Gear::G5,
            Gear::G6,
        ] {
            let f = at(State::Armed(g), 0);
            let (green, red) = shown_code(&f);
            assert_eq!(gear_from_code(green), Some(g));
            assert_eq!(red, 0);
            assert!(f.green[MODE] && !f.red[MODE]);
        }
        assert_eq!(shown_code(&at(State::Armed(Gear::G6), 0)).0, 0b110);
        assert_eq!(shown_code(&at(State::Armed(Gear::N), 0)).0, 0b111);
    }

    #[test]
    fn shifting_is_amber_mode() {
        let f = at(
            State::WaitRelease {
                dest: Gear::G3,
                crosses_neutral: false,
                elapsed_ms: 0,
            },
            0,
        );
        assert_eq!(shown_code(&f).0, 3);
        assert!(f.red[MODE] && f.green[MODE]);
    }

    #[test]
    fn unknown_blinks_and_fault_is_solid() {
        let on = at(State::Unknown(Reason::NeutralOpened), 0);
        let off = at(State::Unknown(Reason::NeutralOpened), 250);
        assert_eq!(shown_code(&on), (0, 3));
        assert!(on.red[MODE] && !off.red[MODE]);
        for t in [0, 250, 500, 750] {
            let f = at(State::Fault(Reason::Readback), t);
            assert_eq!(shown_code(&f), (0, 6));
            assert!(f.red[MODE]);
        }
    }

    #[test]
    fn status_heartbeat_colours() {
        let car = frame(&LedInputs {
            state: State::Armed(Gear::G1),
            neutral: false,
            on_usb: false,
            time_ms: 0,
        });
        assert!(car.green[STAT] && !car.red[STAT]);
        let usb = frame(&LedInputs {
            state: State::Armed(Gear::G1),
            neutral: false,
            on_usb: true,
            time_ms: 0,
        });
        assert!(usb.green[STAT] && usb.red[STAT]);
        let flt = frame(&LedInputs {
            state: State::Fault(Reason::LpsRange),
            neutral: false,
            on_usb: true,
            time_ms: 0,
        });
        assert!(flt.red[STAT] && !flt.green[STAT]);
        let dark = frame(&LedInputs {
            state: State::Armed(Gear::G1),
            neutral: false,
            on_usb: false,
            time_ms: 500,
        });
        assert!(!dark.green[STAT] && !dark.red[STAT]);
    }

    #[test]
    fn neutral_led() {
        let f = frame(&LedInputs {
            state: State::Armed(Gear::N),
            neutral: true,
            on_usb: false,
            time_ms: 0,
        });
        assert!(f.green[NEUT] && !f.red[NEUT]);
        let f = frame(&LedInputs {
            state: State::Unknown(Reason::Disagree),
            neutral: false,
            on_usb: false,
            time_ms: 0,
        });
        assert!(f.red[NEUT] && !f.green[NEUT]);
    }

    #[test]
    fn current_stays_within_budget() {
        // At most 7 dies at once in normal running: N code (3) + MODE (1) + STAT amber (2) + NEUT (1).
        let f = frame(&LedInputs {
            state: State::Armed(Gear::N),
            neutral: true,
            on_usb: true,
            time_ms: 0,
        });
        assert!(f.dies_on() <= 7);
    }

    #[test]
    fn lamp_test_sequence() {
        assert_eq!(lamp_test(0).map(|f| f.red[NEUT]), Some(true));
        assert_eq!(lamp_test(699).map(|f| f.green[0]), Some(true));
        assert!(lamp_test(700).is_none());
        for t in (0..700).step_by(5) {
            assert_eq!(lamp_test(t).map(|f| f.dies_on()), Some(2));
        }
    }
}
