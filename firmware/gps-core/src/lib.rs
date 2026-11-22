//! Gear position sensor logic, with no hardware types. Runs on the board and in `cargo test`.
//!
//! The firmware samples the ADC every 5 ms, fills in a [`Sample`] and calls [`Core::tick`],
//! which returns the DAC code and the LED pattern for that tick.

#![cfg_attr(not(test), no_std)]

pub mod cal;
pub mod config;
pub mod debounce;
pub mod fsm;
pub mod gear;
pub mod led;
pub mod outctl;

use config::Config;
use fsm::{Fsm, Inputs, State, Status};
use led::{LedFrame, LedInputs};
use outctl::{OutCtl, OutInputs, OutOutputs};

/// One tick of raw readings.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    /// Averaged 12-bit reading of `PA0` (LPS divider).
    pub lps_raw: u16,
    /// Averaged 12-bit reading of `PA1` (gear output readback divider).
    pub readback_raw: u16,
    /// Averaged 12-bit reading of VREFINT (ADC channel 17).
    pub vrefint_raw: u16,
    /// Factory `VREFINT_CAL` (0x1FFFF7BA).
    pub vrefint_cal: u16,
    /// `PB2` low: neutral switch closed.
    pub neutral_closed: bool,
    /// `PA2` low: running on USB power.
    pub on_usb: bool,
    /// Milliseconds since boot.
    pub time_ms: u32,
}

/// Everything the firmware writes out after a tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Outputs {
    pub dac_code: u16,
    pub leds: LedFrame,
    pub status: Status,
    pub out: OutOutputs,
    pub vdda_mv: u32,
    pub lps_mv: u32,
    pub readback_mv: u32,
}

pub struct Core {
    cfg: Config,
    fsm: Fsm,
    out: OutCtl,
}

impl Core {
    pub fn new(cfg: Config) -> Self {
        Self {
            fsm: Fsm::new(&cfg),
            out: OutCtl::new(&cfg),
            cfg,
        }
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn state(&self) -> State {
        self.fsm.state()
    }

    pub fn tick(&mut self, s: &Sample) -> Outputs {
        let cfg = &self.cfg;
        let vdda = cal::vdda_mv(u32::from(s.vrefint_raw), u32::from(s.vrefint_cal), cfg);
        let vdda_ok = vdda.is_some_and(|v| (cfg.vdda_min_mv..=cfg.vdda_max_mv).contains(&v));
        // With an implausible VDDA keep converting at the nominal value; reason 7 follows.
        let vdda_mv = if vdda_ok { vdda.unwrap_or(3300) } else { 3300 };
        let lps_mv = cal::lps_mv(cal::pin_mv(u32::from(s.lps_raw), vdda_mv), cfg);
        let readback_mv = cal::readback_mv(cal::pin_mv(u32::from(s.readback_raw), vdda_mv), cfg);

        let status = self.fsm.step(
            &Inputs {
                lps_mv,
                neutral_closed: s.neutral_closed,
                vdda_ok,
                output_fault: self.out.fault(),
            },
            cfg,
        );
        if status.clear_output_fault {
            self.out.clear_fault();
        }
        let out = self.out.step(
            &OutInputs {
                target_mv: status.target_mv,
                gear: status.gear,
                readback_mv,
                vdda_mv,
                vdda_ok,
            },
            cfg,
        );
        let leds = led::lamp_test(s.time_ms).unwrap_or_else(|| {
            led::frame(&LedInputs {
                state: status.state,
                neutral: status.neutral,
                on_usb: s.on_usb,
                time_ms: s.time_ms,
            })
        });
        Outputs {
            dac_code: out.code,
            leds,
            status,
            out,
            vdda_mv,
            lps_mv,
            readback_mv,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gear::Gear;

    /// Raw ADC count for a sensor-side LPS voltage at 3.3 V VDDA.
    fn lps_raw(mv: u32) -> u16 {
        (mv * 15 / 25 * 4095 / 3300) as u16
    }

    #[test]
    fn end_to_end_shift_and_latency() {
        let mut core = Core::new(config::NOMINAL);
        let mut t = 0;
        let mut tick = |core: &mut Core, lps: u32, n: bool, rb: u16| {
            t += 5;
            core.tick(&Sample {
                lps_raw: lps_raw(lps),
                readback_raw: rb,
                vrefint_raw: 1527,
                vrefint_cal: 1527,
                neutral_closed: n,
                on_usb: false,
                time_ms: t,
            })
        };
        // Arm in N; the readback follows the command (ideal plant).
        let mut o = tick(&mut core, 2200, true, 0);
        for _ in 0..200 {
            let rb = (o.status.target_mv * 15 / 25 * 4095 / 3300) as u16;
            o = tick(&mut core, 2200, true, rb);
        }
        assert_eq!(core.state(), State::Armed(Gear::N));
        assert!(!o.out.fault);
        // Upshift: the output changes on the third sample above 3.0 V (15 ms debounce).
        let rb = (o.status.target_mv * 15 / 25 * 4095 / 3300) as u16;
        let mut n = 0;
        loop {
            n += 1;
            o = tick(&mut core, 3300, false, rb);
            if o.status.gear == Some(Gear::G2) {
                break;
            }
            assert!(n < 10);
        }
        assert_eq!(n, 3);
        assert_eq!(o.status.target_mv, 1700);
        assert!(o.dac_code > 1000);
    }
}
