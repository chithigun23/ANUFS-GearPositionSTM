//! Gears and the shift order 1, N, 2, 3, 4, 5, 6.

/// A gear position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Gear {
    N,
    G1,
    G2,
    G3,
    G4,
    G5,
    G6,
}

impl Gear {
    /// Index into the voltage and trim tables: N is 0, gears 1 to 6 are 1 to 6.
    pub const fn index(self) -> usize {
        match self {
            Gear::N => 0,
            Gear::G1 => 1,
            Gear::G2 => 2,
            Gear::G3 => 3,
            Gear::G4 => 4,
            Gear::G5 => 5,
            Gear::G6 => 6,
        }
    }

    /// Value shown on the code LEDs (LED3 LED2 LED1 = 4 2 1): the gear number, and 7 for N.
    pub const fn led_code(self) -> u8 {
        match self {
            Gear::N => 7,
            g => g.index() as u8,
        }
    }
}

/// Where a shift goes, decided when it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shift {
    /// The full-shift destination, output as soon as the shift is seen.
    pub dest: Gear,
    /// The shift starts on 1, N or 2 and passes the N position: the neutral switch decides the
    /// final gear at the post-shift check (closed means N, open means `dest`).
    pub crosses_neutral: bool,
}

/// Upshift. 1 and N go to 2 through N; 6 stays 6.
pub const fn up(from: Gear) -> Shift {
    match from {
        Gear::G1 | Gear::N => Shift {
            dest: Gear::G2,
            crosses_neutral: true,
        },
        Gear::G2 => Shift {
            dest: Gear::G3,
            crosses_neutral: false,
        },
        Gear::G3 => Shift {
            dest: Gear::G4,
            crosses_neutral: false,
        },
        Gear::G4 => Shift {
            dest: Gear::G5,
            crosses_neutral: false,
        },
        Gear::G5 | Gear::G6 => Shift {
            dest: Gear::G6,
            crosses_neutral: false,
        },
    }
}

/// Downshift. 2 and N go to 1 through N; 1 stays 1.
pub const fn down(from: Gear) -> Shift {
    match from {
        Gear::G2 | Gear::N => Shift {
            dest: Gear::G1,
            crosses_neutral: true,
        },
        Gear::G1 => Shift {
            dest: Gear::G1,
            crosses_neutral: false,
        },
        Gear::G3 => Shift {
            dest: Gear::G2,
            crosses_neutral: false,
        },
        Gear::G4 => Shift {
            dest: Gear::G3,
            crosses_neutral: false,
        },
        Gear::G5 => Shift {
            dest: Gear::G4,
            crosses_neutral: false,
        },
        Gear::G6 => Shift {
            dest: Gear::G5,
            crosses_neutral: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_and_clamps() {
        assert_eq!(
            up(Gear::G1),
            Shift {
                dest: Gear::G2,
                crosses_neutral: true
            }
        );
        assert_eq!(up(Gear::N).dest, Gear::G2);
        assert_eq!(up(Gear::G5).dest, Gear::G6);
        assert_eq!(up(Gear::G6).dest, Gear::G6);
        assert_eq!(
            down(Gear::G2),
            Shift {
                dest: Gear::G1,
                crosses_neutral: true
            }
        );
        assert_eq!(down(Gear::N).dest, Gear::G1);
        assert_eq!(down(Gear::G1).dest, Gear::G1);
        assert_eq!(down(Gear::G6).dest, Gear::G5);
        assert!(!up(Gear::G3).crosses_neutral);
    }

    #[test]
    fn led_codes() {
        assert_eq!(Gear::N.led_code(), 0b111);
        assert_eq!(Gear::G1.led_code(), 0b001);
        assert_eq!(Gear::G6.led_code(), 0b110);
    }
}
