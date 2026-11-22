//! Calibration math: ADC counts to volts, VDDA from VREFINT, voltage to DAC code.
//!
//! Integer millivolts throughout. The DAC coefficient is computed once (`OutputCal::new`).

use crate::config::Config;

/// 12-bit full scale.
pub const FULL_SCALE: u32 = 4095;
/// VREFINT_CAL is taken at this VDDA (STM32F072 datasheet).
pub const VREFINT_CAL_VDDA_MV: u32 = 3300;

/// VDDA from the averaged VREFINT reading and the factory `VREFINT_CAL` word.
/// `None` if the reading is zero.
pub fn vdda_mv(vrefint_raw: u32, vrefint_cal: u32, cfg: &Config) -> Option<u32> {
    if vrefint_raw == 0 {
        return None;
    }
    let num =
        u64::from(VREFINT_CAL_VDDA_MV) * u64::from(vrefint_cal) * u64::from(cfg.vdda_trim_ppm);
    let den = u64::from(vrefint_raw) * 1_000_000;
    u32::try_from((num + den / 2) / den).ok()
}

/// Voltage at an ADC pin.
pub fn pin_mv(raw: u32, vdda_mv: u32) -> u32 {
    (raw * vdda_mv + FULL_SCALE / 2) / FULL_SCALE
}

/// Sensor-side LPS voltage from the voltage at `PA0`.
pub fn lps_mv(pin_mv: u32, cfg: &Config) -> u32 {
    let num = u64::from(pin_mv) * u64::from(cfg.r5 + cfg.r6);
    ((num + u64::from(cfg.r6) / 2) / u64::from(cfg.r6)) as u32
}

/// Gear output voltage at the connector from the voltage at `PA1`.
pub fn readback_mv(pin_mv: u32, cfg: &Config) -> u32 {
    let num = u64::from(pin_mv) * u64::from(cfg.r14 + cfg.r15);
    ((num + u64::from(cfg.r15) / 2) / u64::from(cfg.r15)) as u32
}

/// Feed-forward from connector millivolts to DAC code, fixed-point with 16 fractional bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputCal {
    /// `4095 * (1 + Rs / R_load) / G * 2^16`, where `G = 1 + R11 / R12`.
    k: u64,
}

impl OutputCal {
    pub fn new(cfg: &Config) -> Self {
        // Rs = R29 || R30, R_load = (R14 + R15) || R_ECU, as exact fractions.
        let (r29, r30) = (u128::from(cfg.r29), u128::from(cfg.r30));
        let rs_num = r29 * r30;
        let rs_den = r29 + r30;
        let (a, b) = (u128::from(cfg.r14 + cfg.r15), u128::from(cfg.r_ecu));
        let rl_num = a * b;
        let rl_den = a + b;
        // (1 + Rs/RL) / G = (RL + Rs) / RL * R12 / (R12 + R11)
        let num =
            (u128::from(FULL_SCALE) * (rl_num * rs_den + rs_num * rl_den) * u128::from(cfg.r12))
                << 16;
        let den = rl_num * rs_den * u128::from(cfg.r12 + cfg.r11);
        Self {
            k: ((num + den / 2) / den) as u64,
        }
    }

    /// DAC code for a connector voltage at the given VDDA, before trim.
    /// 0 mV gives code 0 exactly. Returns the code and whether it was clamped.
    pub fn code(&self, target_mv: u32, vdda_mv: u32) -> (u16, bool) {
        if target_mv == 0 || vdda_mv == 0 {
            return (0, false);
        }
        let c = (self.k * u64::from(target_mv) / u64::from(vdda_mv) + (1 << 15)) >> 16;
        if c > u64::from(FULL_SCALE) {
            (FULL_SCALE as u16, true)
        } else {
            (c as u16, false)
        }
    }

    /// Signed DAC codes for a signed error in millivolts (used by the trim).
    pub fn codes_for(&self, err_mv: i32, vdda_mv: u32) -> i32 {
        if vdda_mv == 0 {
            return 0;
        }
        let mag =
            (self.k * u64::from(err_mv.unsigned_abs()) / u64::from(vdda_mv) + (1 << 15)) >> 16;
        let mag = mag.min(i32::MAX as u64) as i32;
        if err_mv < 0 { -mag } else { mag }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NOMINAL;

    fn reference_code(target_mv: f64, vdda_mv: f64, c: &Config) -> f64 {
        let rs = f64::from(c.r29) * f64::from(c.r30) / f64::from(c.r29 + c.r30);
        let a = f64::from(c.r14 + c.r15);
        let rl = a * f64::from(c.r_ecu) / (a + f64::from(c.r_ecu));
        let g = 1.0 + f64::from(c.r11) / f64::from(c.r12);
        4095.0 / vdda_mv * target_mv * (1.0 + rs / rl) / g
    }

    #[test]
    fn dac_code_matches_the_plan_formula() {
        for vdda in [3000u32, 3150, 3300, 3450, 3600] {
            let cal = OutputCal::new(&NOMINAL);
            for &mv in &NOMINAL.gear_mv {
                let (code, clamped) = cal.code(mv, vdda);
                let want = reference_code(f64::from(mv), f64::from(vdda), &NOMINAL).round();
                assert!(
                    (f64::from(code) - want).abs() <= 1.0,
                    "{mv} mV at {vdda}: {code} vs {want}"
                );
                assert!(!clamped);
            }
        }
    }

    #[test]
    fn dac_code_follows_altered_resistors() {
        let mut c = NOMINAL;
        c.r11 = 5_100;
        c.r12 = 9_900;
        c.r_ecu = 100_000;
        let cal = OutputCal::new(&c);
        let (code, _) = cal.code(4100, 3300);
        let want = reference_code(4100.0, 3300.0, &c).round();
        assert!((f64::from(code) - want).abs() <= 1.0);
    }

    #[test]
    fn nominal_gear_six_code() {
        // Hand calculation: 4095 / 3300 * 4100 * (1 + 195 / 24390.2) / 1.499 = 3421.3
        let (code, _) = OutputCal::new(&NOMINAL).code(4100, 3300);
        assert_eq!(code, 3421);
    }

    #[test]
    fn zero_volts_is_code_zero_and_overrange_clamps() {
        let cal = OutputCal::new(&NOMINAL);
        assert_eq!(cal.code(0, 3300), (0, false));
        assert_eq!(cal.code(6000, 3000), (4095, true));
    }

    #[test]
    fn vdda_from_vrefint() {
        // VREFINT_CAL of 1527 read back as 1527 means VDDA is exactly 3.3 V.
        assert_eq!(vdda_mv(1527, 1527, &NOMINAL), Some(3300));
        // A lower reading means a higher VDDA.
        assert_eq!(vdda_mv(1400, 1527, &NOMINAL), Some(3599));
        assert_eq!(vdda_mv(0, 1527, &NOMINAL), None);
        let mut c = NOMINAL;
        c.vdda_trim_ppm = 1_010_000;
        assert_eq!(vdda_mv(1527, 1527, &c), Some(3333));
    }

    #[test]
    fn dividers() {
        assert_eq!(pin_mv(4095, 3300), 3300);
        assert_eq!(lps_mv(3000, &NOMINAL), 5000);
        assert_eq!(readback_mv(2460, &NOMINAL), 4100);
    }
}
