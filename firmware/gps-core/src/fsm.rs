//! Shift state machine: UNKNOWN, ARMED, WAIT_RELEASE and FAULT, with the trust model from
//! the firmware plan (docs/firmware-plan.md, "State machine and faults").

use crate::config::Config;
use crate::debounce::Debouncer;
use crate::gear::{self, Gear};

/// Why the output is at 0 V. The number is shown in red on the code LEDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Reason {
    /// 1: not known at start-up.
    StartUp = 1,
    /// 2: the gear count and the neutral switch disagree.
    Disagree = 2,
    /// 3: the neutral switch opened with no shift.
    NeutralOpened = 3,
    /// 4: the LPS is out of range.
    LpsRange = 4,
    /// 5: the LPS did not return to rest after a shift.
    NoRelease = 5,
    /// 6: the output readback does not match the command.
    Readback = 6,
    /// 7: VDDA (or VREFINT) is implausible.
    Vdda = 7,
}

/// Where the LPS sits, sensor-side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Zone {
    Invalid,
    Down,
    Rest,
    Up,
}

pub fn zone(lps_mv: u32, cfg: &Config) -> Zone {
    if lps_mv < cfg.lps_min_mv || lps_mv > cfg.lps_max_mv {
        Zone::Invalid
    } else if lps_mv >= cfg.upshift_mv {
        Zone::Up
    } else if lps_mv <= cfg.downshift_mv {
        Zone::Down
    } else {
        Zone::Rest
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum State {
    Unknown(Reason),
    Armed(Gear),
    WaitRelease {
        dest: Gear,
        crosses_neutral: bool,
        elapsed_ms: u32,
    },
    Fault(Reason),
}

/// One sample of the inputs, already converted to engineering units.
#[derive(Clone, Copy, Debug)]
pub struct Inputs {
    /// Sensor-side LPS voltage.
    pub lps_mv: u32,
    /// Raw (not debounced) neutral switch: true when closed to ground.
    pub neutral_closed: bool,
    /// VDDA inside its plausible range on this sample.
    pub vdda_ok: bool,
    /// The output controller has latched a readback fault.
    pub output_fault: bool,
}

/// What the rest of the firmware needs from one step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Status {
    pub state: State,
    /// Gear voltage to output; 0 in UNKNOWN and FAULT.
    pub target_mv: u32,
    /// Gear being output (for the trim table and the LEDs).
    pub gear: Option<Gear>,
    /// Debounced neutral switch.
    pub neutral: bool,
    /// Set on the step that leaves a readback fault: clear the output controller's latch.
    pub clear_output_fault: bool,
}

pub struct Fsm {
    state: State,
    zone: Debouncer<Zone>,
    neutral: Debouncer<bool>,
    vdda_bad_ms: u32,
    neutral_open_ms: u32,
    ticks: u32,
}

impl Fsm {
    pub const fn new(cfg: &Config) -> Self {
        Self {
            state: State::Unknown(Reason::StartUp),
            // Start from Invalid so the LPS must be seen valid for the debounce time.
            zone: Debouncer::new(Zone::Invalid, cfg.debounce_ticks),
            neutral: Debouncer::new(false, cfg.debounce_ticks),
            vdda_bad_ms: 0,
            neutral_open_ms: 0,
            ticks: 0,
        }
    }

    pub const fn state(&self) -> State {
        self.state
    }

    pub fn step(&mut self, inp: &Inputs, cfg: &Config) -> Status {
        let zone = self.zone.update(zone(inp.lps_mv, cfg));
        let neutral = self.neutral.update(inp.neutral_closed);
        self.vdda_bad_ms = if inp.vdda_ok {
            0
        } else {
            self.vdda_bad_ms.saturating_add(cfg.tick_ms)
        };
        let vdda_fault = self.vdda_bad_ms >= cfg.vdda_fault_ms;
        let mut clear_output_fault = false;
        self.ticks = self.ticks.saturating_add(1);
        let booting = self.ticks <= u32::from(cfg.debounce_ticks);

        // Faults that apply in every state, most important first.
        let fault = if vdda_fault {
            Some(Reason::Vdda)
        } else if zone == Zone::Invalid && !booting {
            Some(Reason::LpsRange)
        } else if inp.output_fault && self.state != State::Fault(Reason::Readback) {
            // While FAULT 6 is showing, the latch stays set until neutral is seen below.
            Some(Reason::Readback)
        } else {
            None
        };

        self.state = match (self.state, fault) {
            // An active fault holds or replaces the state. A readback fault never replaces a
            // more serious one that is already showing.
            (State::Fault(r), Some(f)) if (r as u8) >= (f as u8) => State::Fault(r),
            (_, Some(f)) => State::Fault(f),

            // Leaving UNKNOWN or FAULT: LPS valid and at rest, neutral closed, both debounced.
            (State::Unknown(_) | State::Fault(_), None) => {
                if zone == Zone::Rest && neutral {
                    if matches!(self.state, State::Fault(Reason::Readback)) {
                        clear_output_fault = true;
                    }
                    self.neutral_open_ms = 0;
                    State::Armed(Gear::N)
                } else {
                    self.state
                }
            }

            (State::Armed(g), None) => self.armed(g, zone, neutral, cfg),

            (
                State::WaitRelease {
                    dest,
                    crosses_neutral,
                    elapsed_ms,
                },
                None,
            ) => {
                let elapsed_ms = elapsed_ms.saturating_add(cfg.tick_ms);
                if zone == Zone::Rest && elapsed_ms >= cfg.settle_ms {
                    post_shift_check(dest, crosses_neutral, neutral)
                } else if elapsed_ms >= cfg.release_timeout_ms {
                    State::Fault(Reason::NoRelease)
                } else {
                    State::WaitRelease {
                        dest,
                        crosses_neutral,
                        elapsed_ms,
                    }
                }
            }
        };

        let gear = match self.state {
            State::Armed(g) => Some(g),
            State::WaitRelease { dest, .. } => Some(dest),
            _ => None,
        };
        Status {
            state: self.state,
            target_mv: gear.map_or(0, |g| cfg.gear_mv[g.index()]),
            gear,
            neutral,
            clear_output_fault,
        }
    }

    fn armed(&mut self, g: Gear, zone: Zone, neutral: bool, cfg: &Config) -> State {
        // A shift starts on the tick the debounced LPS leaves the rest band; the new gear is
        // output in the same tick.
        let shift = match zone {
            Zone::Up => Some(gear::up(g)),
            Zone::Down => Some(gear::down(g)),
            _ => None,
        };
        if let Some(s) = shift {
            self.neutral_open_ms = 0;
            return State::WaitRelease {
                dest: s.dest,
                crosses_neutral: s.crosses_neutral,
                elapsed_ms: 0,
            };
        }
        // At rest. The neutral switch is trusted: closed means N.
        if neutral {
            self.neutral_open_ms = 0;
            return State::Armed(Gear::N);
        }
        if g == Gear::N {
            // Neutral switch open at rest with no shift.
            self.neutral_open_ms = self.neutral_open_ms.saturating_add(cfg.tick_ms);
            if self.neutral_open_ms >= cfg.neutral_open_ms {
                self.neutral_open_ms = 0;
                return if cfg.assume_first_when_leaving_neutral {
                    State::Armed(Gear::G1)
                } else {
                    State::Unknown(Reason::NeutralOpened)
                };
            }
        } else {
            self.neutral_open_ms = 0;
        }
        State::Armed(g)
    }
}

/// LPS back at rest and the settle time passed: decide the final gear.
fn post_shift_check(dest: Gear, crosses_neutral: bool, neutral: bool) -> State {
    let g = if crosses_neutral && neutral {
        Gear::N
    } else if neutral {
        // Gear not N but the switch is closed: the switch is trusted.
        Gear::N
    } else {
        dest
    };
    if g == Gear::N && !neutral {
        // The count says N but the switch is open: cannot tell which gear it is.
        State::Unknown(Reason::Disagree)
    } else {
        State::Armed(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NOMINAL;

    const REST: u32 = 2200;
    const UP: u32 = 3300;
    const DOWN: u32 = 1200;

    struct Rig {
        fsm: Fsm,
        last: Option<Status>,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                fsm: Fsm::new(&NOMINAL),
                last: None,
            }
        }
        fn run(&mut self, lps: u32, neutral: bool, ticks: u32) -> Status {
            for _ in 0..ticks {
                let s = self.fsm.step(
                    &Inputs {
                        lps_mv: lps,
                        neutral_closed: neutral,
                        vdda_ok: true,
                        output_fault: false,
                    },
                    &NOMINAL,
                );
                self.last = Some(s);
            }
            self.last.unwrap()
        }
        fn arm(&mut self) {
            let s = self.run(REST, true, 3);
            assert_eq!(s.state, State::Armed(Gear::N));
        }
        /// A complete shift: out to `lps` for 40 ms, back to rest for 100 ms.
        fn shift(&mut self, lps: u32, neutral_after: bool) -> Status {
            self.run(lps, false, 8);
            self.run(REST, neutral_after, 20)
        }
    }

    #[test]
    fn starts_unknown_and_arms_on_neutral_at_rest() {
        let mut r = Rig::new();
        let s = r.run(REST, false, 10);
        assert_eq!(s.state, State::Unknown(Reason::StartUp));
        assert_eq!(s.target_mv, 0);
        // Needs both debounced: two samples are not enough.
        assert_eq!(r.run(REST, true, 2).state, State::Unknown(Reason::StartUp));
        let s = r.run(REST, true, 1);
        assert_eq!(s.state, State::Armed(Gear::N));
        assert_eq!(s.target_mv, 500);
    }

    #[test]
    fn unplugged_lps_at_power_up_is_reason_4() {
        let mut r = Rig::new();
        assert_eq!(r.run(0, true, 3).state, State::Unknown(Reason::StartUp));
        assert_eq!(r.run(0, true, 1).state, State::Fault(Reason::LpsRange));
    }

    #[test]
    fn does_not_arm_while_lps_is_moving() {
        let mut r = Rig::new();
        assert_eq!(r.run(UP, true, 10).state, State::Unknown(Reason::StartUp));
    }

    #[test]
    fn shift_output_changes_on_the_debounced_crossing() {
        let mut r = Rig::new();
        r.arm();
        r.run(REST, false, 0);
        assert_eq!(r.run(UP, false, 2).target_mv, 500);
        let s = r.run(UP, false, 1);
        assert_eq!(s.gear, Some(Gear::G2));
        assert_eq!(s.target_mv, 1700);
        assert!(matches!(s.state, State::WaitRelease { .. }));
    }

    #[test]
    fn walks_up_and_down_the_box() {
        let mut r = Rig::new();
        r.arm();
        // N to 2 (switch open at rest), then 3 to 6 and clamp at 6.
        assert_eq!(r.shift(UP, false).state, State::Armed(Gear::G2));
        for g in [Gear::G3, Gear::G4, Gear::G5, Gear::G6, Gear::G6] {
            assert_eq!(r.shift(UP, false).state, State::Armed(g));
        }
        for g in [Gear::G5, Gear::G4, Gear::G3, Gear::G2] {
            assert_eq!(r.shift(DOWN, false).state, State::Armed(g));
        }
        // Full shift 2 to 1 through N: switch open at the end means 1.
        assert_eq!(r.shift(DOWN, false).state, State::Armed(Gear::G1));
        // Clamp at 1.
        assert_eq!(r.shift(DOWN, false).state, State::Armed(Gear::G1));
    }

    #[test]
    fn half_shift_into_neutral_is_decided_by_the_switch() {
        let mut r = Rig::new();
        r.arm();
        r.shift(DOWN, false); // N to 1
        let s = r.shift(UP, true); // 1 up, stops in N with the switch closed
        assert_eq!(s.state, State::Armed(Gear::N));
        let s = r.shift(UP, false); // N to 2
        assert_eq!(s.state, State::Armed(Gear::G2));
    }

    #[test]
    fn switch_is_ignored_while_moving() {
        let mut r = Rig::new();
        r.arm();
        r.shift(DOWN, false); // 1
        // Upshift 1 to 2 passes N: switch closes briefly mid-shift, open at rest.
        r.run(UP, false, 4);
        r.run(UP, true, 4);
        let s = r.run(REST, false, 20);
        assert_eq!(s.state, State::Armed(Gear::G2));
    }

    #[test]
    fn neutral_switch_forces_neutral_at_rest() {
        let mut r = Rig::new();
        r.arm();
        r.shift(UP, false);
        r.shift(UP, false); // 3
        assert_eq!(r.run(REST, true, 3).state, State::Armed(Gear::N));
    }

    #[test]
    fn neutral_opening_without_a_shift_is_reason_3() {
        let mut r = Rig::new();
        r.arm();
        // 15 ms to debounce the switch opening, then 100 ms open at rest.
        let s = r.run(REST, false, 3 + 18);
        assert_eq!(s.state, State::Armed(Gear::N));
        let s = r.run(REST, false, 1);
        assert_eq!(s.state, State::Unknown(Reason::NeutralOpened));
        assert_eq!(s.target_mv, 0);
        // Recovers when neutral is seen again.
        assert_eq!(r.run(REST, true, 3).state, State::Armed(Gear::N));
    }

    #[test]
    fn assume_first_option() {
        let mut c = NOMINAL;
        c.assume_first_when_leaving_neutral = true;
        let mut f = Fsm::new(&c);
        let mut s = None;
        for (n, t) in [(true, 3), (false, 30)] {
            for _ in 0..t {
                s = Some(f.step(
                    &Inputs {
                        lps_mv: REST,
                        neutral_closed: n,
                        vdda_ok: true,
                        output_fault: false,
                    },
                    &c,
                ));
            }
        }
        assert_eq!(s.unwrap().state, State::Armed(Gear::G1));
    }

    #[test]
    fn no_release_is_reason_5() {
        let mut r = Rig::new();
        r.arm();
        // 3 samples to see the shift, then 2 s waiting.
        let s = r.run(UP, false, 3 + 399);
        assert!(matches!(s.state, State::WaitRelease { .. }));
        let s = r.run(UP, false, 1);
        assert_eq!(s.state, State::Fault(Reason::NoRelease));
        assert_eq!(s.target_mv, 0);
    }

    #[test]
    fn lps_out_of_range_is_reason_4_after_debounce() {
        let mut r = Rig::new();
        r.arm();
        r.shift(UP, false);
        assert_eq!(r.run(100, false, 2).state, State::Armed(Gear::G2));
        let s = r.run(100, false, 1);
        assert_eq!(s.state, State::Fault(Reason::LpsRange));
        // Back in range but neutral open: stays in FAULT.
        assert_eq!(r.run(REST, false, 10).state, State::Fault(Reason::LpsRange));
        assert_eq!(r.run(REST, true, 3).state, State::Armed(Gear::N));
    }

    #[test]
    fn vdda_fault_after_50_ms() {
        let mut f = Fsm::new(&NOMINAL);
        let inp = |ok| Inputs {
            lps_mv: REST,
            neutral_closed: true,
            vdda_ok: ok,
            output_fault: false,
        };
        for _ in 0..3 {
            f.step(&inp(true), &NOMINAL);
        }
        for _ in 0..9 {
            assert_eq!(f.step(&inp(false), &NOMINAL).state, State::Armed(Gear::N));
        }
        assert_eq!(
            f.step(&inp(false), &NOMINAL).state,
            State::Fault(Reason::Vdda)
        );
        // Clears when VDDA is good, the LPS is at rest and neutral is closed.
        assert_eq!(f.step(&inp(true), &NOMINAL).state, State::Armed(Gear::N));
    }

    #[test]
    fn readback_fault_latches_until_neutral() {
        let mut f = Fsm::new(&NOMINAL);
        let inp = |n, of| Inputs {
            lps_mv: REST,
            neutral_closed: n,
            vdda_ok: true,
            output_fault: of,
        };
        for _ in 0..3 {
            f.step(&inp(true, false), &NOMINAL);
        }
        for _ in 0..30 {
            f.step(&inp(false, false), &NOMINAL);
        }
        // (now reason 3); the output controller reports a fault
        let s = f.step(&inp(false, true), &NOMINAL);
        assert_eq!(s.state, State::Fault(Reason::Readback));
        for _ in 0..2 {
            f.step(&inp(true, true), &NOMINAL);
        }
        let s = f.step(&inp(true, true), &NOMINAL);
        // Neutral seen at rest: back to N, and the controller is told to clear its latch.
        assert_eq!(s.state, State::Armed(Gear::N));
        assert!(s.clear_output_fault);
    }

    #[test]
    fn output_is_always_a_table_value_or_zero() {
        let mut r = Rig::new();
        let mut seed = 0x1234_5678u32;
        for _ in 0..20_000 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let lps = seed % 5200;
            let n = seed & 0x100 != 0;
            let s = r.run(lps, n, 1);
            assert!(s.target_mv == 0 || NOMINAL.gear_mv.contains(&s.target_mv));
        }
    }
}
