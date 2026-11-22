//! Gear output control: feed-forward DAC code, per-gear RAM trim from the readback, and the
//! readback fault (reason 6).

use crate::cal::OutputCal;
use crate::config::Config;
use crate::gear::Gear;

#[derive(Clone, Copy, Debug)]
pub struct OutInputs {
    /// Commanded connector voltage; 0 means code 0 exactly.
    pub target_mv: u32,
    /// Gear being output, selects the trim slot.
    pub gear: Option<Gear>,
    /// Connector voltage measured through `R14`/`R15` on `PA1`.
    pub readback_mv: u32,
    pub vdda_mv: u32,
    pub vdda_ok: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct OutOutputs {
    pub code: u16,
    /// Warning: the code hit 0 or 4095 (gear 6 on a low USB rail, for example).
    pub clamped: bool,
    /// Warning: the trim for this gear is at its limit.
    pub trim_at_limit: bool,
    /// Readback fault latched: the output is held at 0 V.
    pub fault: bool,
}

pub struct OutCtl {
    cal: OutputCal,
    trim: [i32; 7],
    last: Option<(u32, Option<Gear>)>,
    since_change_ms: u32,
    acc_mv: u64,
    acc_n: u8,
    bad_ms: u32,
    latched: bool,
}

impl OutCtl {
    pub fn new(cfg: &Config) -> Self {
        Self {
            cal: OutputCal::new(cfg),
            trim: [0; 7],
            last: None,
            since_change_ms: 0,
            acc_mv: 0,
            acc_n: 0,
            bad_ms: 0,
            latched: false,
        }
    }

    /// Release the latch (the state machine has seen neutral).
    pub fn clear_fault(&mut self) {
        self.latched = false;
        self.bad_ms = 0;
        self.since_change_ms = 0;
    }

    pub fn fault(&self) -> bool {
        self.latched
    }

    pub fn trim(&self, g: Gear) -> i32 {
        self.trim[g.index()]
    }

    pub fn step(&mut self, inp: &OutInputs, cfg: &Config) -> OutOutputs {
        let target = if self.latched { 0 } else { inp.target_mv };
        let key = (target, inp.gear);
        if self.last != Some(key) {
            // New command: written this tick, so the shift path never waits on the loop.
            self.last = Some(key);
            self.since_change_ms = 0;
            self.acc_mv = 0;
            self.acc_n = 0;
            self.bad_ms = 0;
        } else {
            self.since_change_ms = self.since_change_ms.saturating_add(cfg.tick_ms);
        }

        let limit = self.cal.codes_for(cfg.trim_limit_mv as i32, inp.vdda_mv);
        let slot = inp.gear.map(Gear::index);

        // ---- readback fault ----
        if !self.latched && self.since_change_ms >= cfg.rb_blank_ms {
            let bad = if target == 0 {
                inp.readback_mv > cfg.rb_zero_mv
            } else {
                inp.readback_mv.abs_diff(target) > cfg.rb_error_mv
            };
            self.bad_ms = if bad {
                self.bad_ms.saturating_add(cfg.tick_ms)
            } else {
                0
            };
            if self.bad_ms >= cfg.rb_fault_ms {
                self.latched = true;
            }
        }
        if self.latched {
            return OutOutputs {
                code: 0,
                clamped: false,
                trim_at_limit: false,
                fault: true,
            };
        }

        // ---- trim ----
        if let Some(i) = slot
            && target > 0
            && inp.vdda_ok
            && self.since_change_ms >= cfg.trim_steady_ms
        {
            self.acc_mv += u64::from(inp.readback_mv);
            self.acc_n += 1;
            if self.acc_n >= cfg.trim_avg_ticks {
                let avg = (self.acc_mv / u64::from(self.acc_n)) as i64;
                let err = i64::from(target) - avg;
                if err.unsigned_abs() > u64::from(cfg.trim_deadband_mv) {
                    let step = self
                        .cal
                        .codes_for(
                            err.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32,
                            inp.vdda_mv,
                        )
                        .clamp(-cfg.trim_max_step, cfg.trim_max_step);
                    // Never step by zero once outside the deadband.
                    let step = if step == 0 { err.signum() as i32 } else { step };
                    self.trim[i] = (self.trim[i] + step).clamp(-limit, limit);
                }
                self.acc_mv = 0;
                self.acc_n = 0;
            }
        }

        // ---- feed-forward plus trim ----
        if target == 0 {
            return OutOutputs {
                code: 0,
                clamped: false,
                trim_at_limit: false,
                fault: false,
            };
        }
        let (ff, mut clamped) = self.cal.code(target, inp.vdda_mv);
        let t = slot.map_or(0, |i| self.trim[i]);
        let raw = i32::from(ff) + t;
        let code = raw.clamp(0, 4095);
        clamped |= code != raw;
        OutOutputs {
            code: code as u16,
            clamped,
            trim_at_limit: limit > 0 && t.abs() >= limit,
            fault: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NOMINAL;

    /// The real output: nominal chain with a gain error and an offset.
    struct Plant {
        gain_ppm: i64,
        offset_mv: i64,
        noise: u32,
        seed: u32,
    }

    impl Plant {
        fn output(&mut self, code: u16, vdda: u32) -> u32 {
            // Invert the nominal feed-forward to get what the code would ideally produce.
            let cal = OutputCal::new(&NOMINAL);
            let per = 4100.0 / f64::from(cal.code(4100, vdda).0);
            let ideal = f64::from(code) * per;
            let mut v = ideal * (1.0 + self.gain_ppm as f64 / 1e6) + self.offset_mv as f64;
            if self.noise > 0 {
                self.seed ^= self.seed << 13;
                self.seed ^= self.seed >> 17;
                self.seed ^= self.seed << 5;
                v += f64::from(self.seed % (2 * self.noise + 1)) - f64::from(self.noise);
            }
            v.max(0.0) as u32
        }
    }

    fn run(
        ctl: &mut OutCtl,
        plant: &mut Plant,
        target: u32,
        gear: Gear,
        ticks: u32,
    ) -> (OutOutputs, u32) {
        let mut last = (
            OutOutputs {
                code: 0,
                clamped: false,
                trim_at_limit: false,
                fault: false,
            },
            0,
        );
        let mut rb = 0;
        for _ in 0..ticks {
            let o = ctl.step(
                &OutInputs {
                    target_mv: target,
                    gear: Some(gear),
                    readback_mv: rb,
                    vdda_mv: 3300,
                    vdda_ok: true,
                },
                &NOMINAL,
            );
            rb = plant.output(o.code, 3300);
            last = (o, rb);
        }
        last
    }

    #[test]
    fn trim_converges_inside_the_deadband() {
        let mut ctl = OutCtl::new(&NOMINAL);
        let mut plant = Plant {
            gain_ppm: -20_000,
            offset_mv: 25,
            noise: 2,
            seed: 7,
        };
        let (o, rb) = run(&mut ctl, &mut plant, 2900, Gear::G4, 400);
        assert!(!o.fault);
        assert!(rb.abs_diff(2900) <= 10, "readback {rb}");
    }

    #[test]
    fn trim_is_kept_per_gear() {
        let mut ctl = OutCtl::new(&NOMINAL);
        let mut plant = Plant {
            gain_ppm: 30_000,
            offset_mv: 0,
            noise: 0,
            seed: 1,
        };
        run(&mut ctl, &mut plant, 3500, Gear::G5, 400);
        let t5 = ctl.trim(Gear::G5);
        assert!(t5 < 0);
        run(&mut ctl, &mut plant, 1100, Gear::G1, 400);
        assert_eq!(ctl.trim(Gear::G5), t5);
    }

    #[test]
    fn trim_is_limited() {
        let mut ctl = OutCtl::new(&NOMINAL);
        let mut plant = Plant {
            gain_ppm: 0,
            offset_mv: -400,
            noise: 0,
            seed: 1,
        };
        // 400 mV low is beyond the 250 mV trim range, and beyond the 150 mV fault threshold:
        // the readback fault trips first.
        let (o, _) = run(&mut ctl, &mut plant, 2300, Gear::G3, 400);
        assert!(o.fault);
        let mut ctl = OutCtl::new(&NOMINAL);
        let mut plant = Plant {
            gain_ppm: 0,
            offset_mv: -140,
            noise: 0,
            seed: 1,
        };
        let (o, rb) = run(&mut ctl, &mut plant, 2300, Gear::G3, 600);
        assert!(!o.fault);
        assert!(rb.abs_diff(2300) <= 6);
    }

    #[test]
    fn shorted_output_faults_within_60_ms_of_blanking() {
        let mut ctl = OutCtl::new(&NOMINAL);
        let mut n = 0;
        loop {
            let o = ctl.step(
                &OutInputs {
                    target_mv: 2900,
                    gear: Some(Gear::G4),
                    readback_mv: 0,
                    vdda_mv: 3300,
                    vdda_ok: true,
                },
                &NOMINAL,
            );
            n += 1;
            if o.fault {
                assert_eq!(o.code, 0);
                break;
            }
            assert!(n < 100);
        }
        // Blank 10 ms, then 50 ms of error.
        let ms = n * NOMINAL.tick_ms;
        assert!(ms <= NOMINAL.rb_blank_ms + 60, "{ms} ms");
        // Latched until cleared.
        let o = ctl.step(
            &OutInputs {
                target_mv: 2900,
                gear: Some(Gear::G4),
                readback_mv: 2900,
                vdda_mv: 3300,
                vdda_ok: true,
            },
            &NOMINAL,
        );
        assert!(o.fault);
        ctl.clear_fault();
        assert!(!ctl.fault());
    }

    #[test]
    fn pulled_up_output_at_zero_volts_faults() {
        let mut ctl = OutCtl::new(&NOMINAL);
        let mut fault = false;
        for _ in 0..20 {
            let o = ctl.step(
                &OutInputs {
                    target_mv: 0,
                    gear: None,
                    readback_mv: 1000,
                    vdda_mv: 3300,
                    vdda_ok: true,
                },
                &NOMINAL,
            );
            assert_eq!(o.code, 0);
            fault = o.fault;
        }
        assert!(fault);
    }

    #[test]
    fn command_change_writes_feed_forward_immediately() {
        let mut ctl = OutCtl::new(&NOMINAL);
        let o = ctl.step(
            &OutInputs {
                target_mv: 4100,
                gear: Some(Gear::G6),
                readback_mv: 0,
                vdda_mv: 3300,
                vdda_ok: true,
            },
            &NOMINAL,
        );
        assert_eq!(o.code, 3421);
        let o = ctl.step(
            &OutInputs {
                target_mv: 0,
                gear: None,
                readback_mv: 0,
                vdda_mv: 3300,
                vdda_ok: true,
            },
            &NOMINAL,
        );
        assert_eq!(o.code, 0);
    }
}
