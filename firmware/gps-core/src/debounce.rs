//! Sample-count debouncer: a new value is believed after `n` consecutive samples.

#[derive(Clone, Copy, Debug)]
pub struct Debouncer<T: Copy + PartialEq> {
    stable: T,
    candidate: T,
    count: u8,
    n: u8,
}

impl<T: Copy + PartialEq> Debouncer<T> {
    pub const fn new(initial: T, n: u8) -> Self {
        Self {
            stable: initial,
            candidate: initial,
            count: 0,
            n,
        }
    }

    /// Feed one sample and return the debounced value.
    pub fn update(&mut self, sample: T) -> T {
        if sample == self.stable {
            self.candidate = sample;
            self.count = 0;
        } else if sample == self.candidate {
            self.count = self.count.saturating_add(1);
            if self.count >= self.n {
                self.stable = sample;
                self.count = 0;
            }
        } else {
            self.candidate = sample;
            self.count = 1;
            if self.count >= self.n {
                self.stable = sample;
                self.count = 0;
            }
        }
        self.stable
    }

    pub const fn value(&self) -> T {
        self.stable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_samples() {
        let mut d = Debouncer::new(false, 3);
        assert!(!d.update(true));
        assert!(!d.update(true));
        assert!(d.update(true));
        // A single glitch does not change it.
        assert!(d.update(false));
        assert!(d.update(true));
        // An interrupted run restarts the count.
        assert!(d.update(false));
        assert!(d.update(false));
        assert!(d.update(true));
        assert!(d.update(false));
        assert!(d.update(false));
        assert!(!d.update(false));
    }
}
