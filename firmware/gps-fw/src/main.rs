//! Gear position sensor firmware: STM32F072CB, one 5 ms loop (docs/firmware-plan.md).
//!
//! Every tick: scan the LPS, the gear output readback and VREFINT four times each, run the
//! logic in `gps-core`, write the DAC, update the LEDs every second tick, feed the watchdog.
//! All decisions live in `gps-core`; this file only touches hardware.

#![no_std]
#![no_main]

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_stm32::adc::{self, Adc, SampleTime};
use embassy_stm32::dac::{DacChannel, Value};
use embassy_stm32::gpio::{AnyPin, Input, Level, Output, OutputOpenDrain, Pull, Speed};
use embassy_stm32::time::Hertz;
use embassy_stm32::wdg::IndependentWatchdog;
use embassy_stm32::{Peri, bind_interrupts, pac, peripherals};
use embassy_time::{Duration, Instant, Ticker, Timer};
use gps_core::config::NOMINAL;
use gps_core::led::LedFrame;
use gps_core::{Core, Sample};
use {defmt_rtt as _, panic_probe as _};

/// `PB2` internal pull-up for the neutral switch. Measure the car's neutral wire with the switch
/// open first: if it sits above 3.6 V (a neutral lamp), set this to false.
const NEUTRAL_PULLUP: bool = true;

/// Watchdog timeout. The LSI runs 30 to 60 kHz, so 75 ms nominal is 50 to 100 ms real.
const WATCHDOG_US: u32 = 75_000;

/// ADC sample time: 71.5 cycles, fine for the 6 kohm dividers and VREFINT.
const SAMPLE: SampleTime = SampleTime::CYCLES71_5;

/// Factory VREFINT calibration word, taken at 3.3 V (STM32F072 datasheet).
const VREFINT_CAL_ADDR: usize = 0x1FFF_F7BA;

bind_interrupts!(struct Irqs {
    ADC1_COMP => adc::InterruptHandler<peripherals::ADC1>;
});

fn clocks() -> embassy_stm32::Config {
    use embassy_stm32::rcc::*;
    let mut c = embassy_stm32::Config::default();
    // 24 MHz crystal on HSE (CAN-grade accuracy), PLL x2 to 48 MHz.
    c.rcc.hse = Some(Hse {
        freq: Hertz(24_000_000),
        mode: HseMode::Oscillator,
    });
    c.rcc.pll = Some(Pll {
        src: PllSource::HSE,
        prediv: PllPreDiv::DIV1,
        mul: PllMul::MUL2,
    });
    c.rcc.sys = Sysclk::PLL1_P;
    c
}

fn vrefint_cal() -> u16 {
    // SAFETY: a read-only system-memory word that is always present on the STM32F072.
    #[allow(unsafe_code)]
    unsafe {
        core::ptr::read_volatile(VREFINT_CAL_ADDR as *const u16)
    }
}

/// Twelve open-drain LED cathodes: low lights a die. Index 0 is `LED1`.
struct Leds {
    red: [OutputOpenDrain<'static>; 6],
    green: [OutputOpenDrain<'static>; 6],
}

impl Leds {
    fn show(&mut self, f: &LedFrame) {
        for (pin, on) in self.red.iter_mut().zip(f.red) {
            pin.set_level(if on { Level::Low } else { Level::High });
        }
        for (pin, on) in self.green.iter_mut().zip(f.green) {
            pin.set_level(if on { Level::Low } else { Level::High });
        }
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_stm32::init(clocks());

    // CAN stays silent from the start: PB1 high puts the TCAN337 in silent mode, PB9 (TXD) is
    // left as an input so R7 holds the bus recessive, and the bxCAN clock stays off.
    let _can_silent = Output::new(p.PB1, Level::High, Speed::Low);

    // DAC: code 0 first, output buffer off (so it can reach 0 V), then enable.
    let mut dac = DacChannel::new_blocking(p.DAC1, p.PA4);
    dac.set(Value::Bit12Right(0));
    dac.disable();
    pac::DAC1.cr().modify(|w| w.set_boff(0, true));
    dac.enable();

    // Reset cause: a watchdog reset is a warning only; the output starts at 0 V regardless.
    let csr = pac::RCC.csr().read();
    if csr.iwdgrstf() {
        warn!("reset by the independent watchdog");
    }
    pac::RCC.csr().modify(|w| w.set_rmvf(true));

    let neutral = Input::new(p.PB2, if NEUTRAL_PULLUP { Pull::Up } else { Pull::None });
    let mux_stat = Input::new(p.PA2, Pull::Up); // TPS2116 ST: open-drain, high = car

    let od = |pin: Peri<'static, AnyPin>| OutputOpenDrain::new(pin, Level::High, Speed::Low);
    let mut leds = Leds {
        red: [
            od(p.PB10.into()),
            od(p.PB12.into()),
            od(p.PB14.into()),
            od(p.PA8.into()),
            od(p.PB3.into()),
            od(p.PB5.into()),
        ],
        green: [
            od(p.PB11.into()),
            od(p.PB13.into()),
            od(p.PB15.into()),
            od(p.PA15.into()),
            od(p.PB4.into()),
            od(p.PB6.into()),
        ],
    };

    // The F0 ADC runs from its own 14 MHz HSI14 clock (CKMODE reset value). The HAL assumes it is
    // running but does not start it, so start it before the ADC calibrates.
    pac::RCC.cr2().modify(|w| w.set_hsi14on(true));
    while !pac::RCC.cr2().read().hsi14rdy() {}

    let mut adc = Adc::new(p.ADC1, Irqs);
    let mut vref = adc.enable_vref();
    Timer::after_micros(10).await; // VREFINT start-up
    let mut lps = p.PA0;
    let mut readback = p.PA1;

    // Average VREFINT 64 times before any decision.
    let cal = vrefint_cal();
    let mut acc = 0u32;
    for _ in 0..64 {
        acc += u32::from(adc.read(&mut vref, SAMPLE).await);
    }
    let vdda = gps_core::cal::vdda_mv(acc / 64, u32::from(cal), &NOMINAL);
    info!("VREFINT_CAL {}, VDDA {} mV", cal, vdda);

    let mut wdg = IndependentWatchdog::new(p.IWDG, WATCHDOG_US);
    wdg.unleash();

    let mut core = Core::new(NOMINAL);
    let start = Instant::now();
    let mut ticker = Ticker::every(Duration::from_millis(u64::from(NOMINAL.tick_ms)));
    let mut led_tick = false;
    let mut last_state = core.state();
    info!("running, state {}", last_state);

    loop {
        // 1. Scan IN0, IN1 and IN17 four times and average.
        let (mut a, mut b, mut v) = (0u32, 0u32, 0u32);
        for _ in 0..4 {
            a += u32::from(adc.read(&mut lps, SAMPLE).await);
            b += u32::from(adc.read(&mut readback, SAMPLE).await);
            v += u32::from(adc.read(&mut vref, SAMPLE).await);
        }

        // 2. Digital inputs. 3. Logic.
        let o = core.tick(&Sample {
            lps_raw: (a / 4) as u16,
            readback_raw: (b / 4) as u16,
            vrefint_raw: (v / 4) as u16,
            vrefint_cal: cal,
            neutral_closed: neutral.is_low(),
            on_usb: mux_stat.is_low(),
            time_ms: start.elapsed().as_millis() as u32,
        });

        // 4. DAC.
        dac.set(Value::Bit12Right(o.dac_code));

        // 5. LEDs every second tick (10 ms).
        led_tick = !led_tick;
        if led_tick {
            leds.show(&o.leds);
        }

        if o.status.state != last_state {
            last_state = o.status.state;
            info!(
                "state {} target {} mV code {} lps {} mV readback {} mV vdda {} mV",
                o.status.state, o.status.target_mv, o.dac_code, o.lps_mv, o.readback_mv, o.vdda_mv
            );
        }
        if o.out.clamped || o.out.trim_at_limit {
            warn!("output warning {}", o.out);
        }

        // 6. Watchdog.
        wdg.pet();
        ticker.next().await;
    }
}
