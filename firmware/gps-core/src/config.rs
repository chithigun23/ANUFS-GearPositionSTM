//! Resistor values, VDDA trim and tunables.
//!
//! The defaults are nominal values. Measure the real resistors with a multimeter, edit
//! [`NOMINAL`] and reflash. All voltages are in millivolts, all times in milliseconds.

/// Everything the logic needs to know about the board and the car.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    // ---- resistors, ohms ----
    /// LPS divider, top (`R5`).
    pub r5: u32,
    /// LPS divider, bottom (`R6`).
    pub r6: u32,
    /// Gear output readback divider, top (`R14`).
    pub r14: u32,
    /// Gear output readback divider, bottom (`R15`).
    pub r15: u32,
    /// Op-amp feedback (`R11`).
    pub r11: u32,
    /// Op-amp gain-setting resistor to ground (`R12`).
    pub r12: u32,
    /// Output series resistors (`R29`, `R30`, in parallel).
    pub r29: u32,
    pub r30: u32,
    /// DAC pull-down (`R10`). Not modelled; the trim absorbs its loading.
    pub r10: u32,
    /// M150 gear input impedance (unknown, ask MoTeC).
    pub r_ecu: u32,

    /// Meter-measured VDDA divided by the VDDA the firmware computes, in parts per million.
    pub vdda_trim_ppm: u32,

    // ---- gear output ----
    /// Output per gear: index 0 is N, 1 to 6 are the gears.
    pub gear_mv: [u32; 7],

    // ---- linear position sensor, sensor-side volts ----
    /// At or above this the actuator is upshifting.
    pub upshift_mv: u32,
    /// At or below this the actuator is downshifting.
    pub downshift_mv: u32,
    /// Valid sensor range; outside it for the debounce time is fault reason 4.
    pub lps_min_mv: u32,
    pub lps_max_mv: u32,

    // ---- timing ----
    /// Main loop period.
    pub tick_ms: u32,
    /// Consecutive samples before the LPS zone or the neutral switch is believed.
    pub debounce_ticks: u8,
    /// Minimum time from a shift starting to the post-shift check.
    pub settle_ms: u32,
    /// Fault reason 5 if the LPS has not returned to rest by this time after a shift.
    pub release_timeout_ms: u32,
    /// Neutral switch open at rest with no shift for this long: reason 3.
    pub neutral_open_ms: u32,

    // ---- VDDA plausibility ----
    pub vdda_min_mv: u32,
    pub vdda_max_mv: u32,
    /// VDDA outside the range for this long: fault reason 7.
    pub vdda_fault_ms: u32,

    // ---- closed-loop trim ----
    /// The command must be steady this long before trimming.
    pub trim_steady_ms: u32,
    /// Readback samples averaged per trim update.
    pub trim_avg_ticks: u8,
    /// No trim inside this error.
    pub trim_deadband_mv: u32,
    /// Trim limit either way, at the connector.
    pub trim_limit_mv: u32,
    /// Largest trim change per update, in DAC codes.
    pub trim_max_step: i32,

    // ---- readback fault (reason 6) ----
    /// No fault check for this long after a command change.
    pub rb_blank_ms: u32,
    /// Error that counts as a fault while a gear is commanded.
    pub rb_error_mv: u32,
    /// Readback that counts as a fault while 0 V is commanded.
    pub rb_zero_mv: u32,
    /// The fault condition must last this long.
    pub rb_fault_ms: u32,

    /// Carried over from the Arduino firmware. When true, the neutral switch opening at rest
    /// with no shift means gear 1 (the driver selected it with the clutch). When false it is
    /// reason 3 and the output goes to 0 V.
    pub assume_first_when_leaving_neutral: bool,
}

/// Nominal values from the schematic and the firmware plan.
pub const NOMINAL: Config = Config {
    r5: 10_000,
    r6: 15_000,
    r14: 10_000,
    r15: 15_000,
    r11: 4_990,
    r12: 10_000,
    r29: 390,
    r30: 390,
    r10: 1_000_000,
    r_ecu: 1_000_000,
    vdda_trim_ppm: 1_000_000,
    gear_mv: [500, 1100, 1700, 2300, 2900, 3500, 4100],
    upshift_mv: 3000,
    downshift_mv: 1500,
    lps_min_mv: 200,
    lps_max_mv: 4800,
    tick_ms: 5,
    debounce_ticks: 3,
    settle_ms: 100,
    release_timeout_ms: 2000,
    neutral_open_ms: 100,
    vdda_min_mv: 3000,
    vdda_max_mv: 3600,
    vdda_fault_ms: 50,
    trim_steady_ms: 20,
    trim_avg_ticks: 8,
    trim_deadband_mv: 6,
    trim_limit_mv: 250,
    trim_max_step: 16,
    rb_blank_ms: 10,
    rb_error_mv: 150,
    rb_zero_mv: 200,
    rb_fault_ms: 50,
    assume_first_when_leaving_neutral: false,
};
