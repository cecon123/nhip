//! Equalizer filters and the peak limiter.

use crate::mechanics::{EqKind, EQ_BANDS, EQ_Q};

#[derive(Clone, Copy, Default)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z: [[f32; 2]; 2],
}

impl Biquad {
    /// Coefficients from the Audio EQ Cookbook (Robert Bristow-Johnson).
    fn design(kind: EqKind, hz: f32, gain_db: f32, rate: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w = 2.0 * std::f32::consts::PI * (hz.min(rate * 0.45)) / rate;
        let (sn, cs) = w.sin_cos();
        let (b0, b1, b2, a0, a1, a2);
        match kind {
            EqKind::Peaking => {
                let alpha = sn / (2.0 * EQ_Q);
                b0 = 1.0 + alpha * a;
                b1 = -2.0 * cs;
                b2 = 1.0 - alpha * a;
                a0 = 1.0 + alpha / a;
                a1 = -2.0 * cs;
                a2 = 1.0 - alpha / a;
            }
            EqKind::LowShelf => {
                let alpha = sn / 2.0 * std::f32::consts::SQRT_2;
                let s = 2.0 * a.sqrt() * alpha;
                b0 = a * ((a + 1.0) - (a - 1.0) * cs + s);
                b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cs);
                b2 = a * ((a + 1.0) - (a - 1.0) * cs - s);
                a0 = (a + 1.0) + (a - 1.0) * cs + s;
                a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cs);
                a2 = (a + 1.0) + (a - 1.0) * cs - s;
            }
            EqKind::HighShelf => {
                let alpha = sn / 2.0 * std::f32::consts::SQRT_2;
                let s = 2.0 * a.sqrt() * alpha;
                b0 = a * ((a + 1.0) + (a - 1.0) * cs + s);
                b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cs);
                b2 = a * ((a + 1.0) + (a - 1.0) * cs - s);
                a0 = (a + 1.0) - (a - 1.0) * cs + s;
                a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cs);
                a2 = (a + 1.0) - (a - 1.0) * cs - s;
            }
        }
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            z: [[0.0; 2]; 2],
        }
    }

    #[inline]
    fn process(&mut self, ch: usize, x: f32) -> f32 {
        let z = &mut self.z[ch];
        let y = self.b0 * x + z[0];
        z[0] = self.b1 * x - self.a1 * y + z[1];
        z[1] = self.b2 * x - self.a2 * y;
        y
    }
}

pub struct Equalizer {
    bands: Vec<Biquad>,
    active: bool,
}

impl Equalizer {
    pub fn new() -> Self {
        Self { bands: Vec::new(), active: false }
    }

    pub fn configure(&mut self, enabled: bool, gains: &[f32; 6], rate: f32) {
        self.active = enabled && gains.iter().any(|g| g.abs() > 0.01);
        if !self.active {
            self.bands.clear();
            return;
        }
        let old = std::mem::take(&mut self.bands);
        self.bands = EQ_BANDS
            .iter()
            .zip(gains)
            .enumerate()
            .map(|(i, (band, &g))| {
                let mut b = Biquad::design(band.kind, band.hz, g, rate);
                if let Some(prev) = old.get(i) {
                    b.z = prev.z;
                }
                b
            })
            .collect();
    }

    pub fn process(&mut self, stereo: &mut [f32]) {
        if !self.active {
            return;
        }
        for frame in stereo.chunks_exact_mut(2) {
            for band in &mut self.bands {
                frame[0] = band.process(0, frame[0]);
                frame[1] = band.process(1, frame[1]);
            }
        }
    }
}

/// Keeps the signal under full scale after boosts: reacts at once, recovers slowly.
pub struct Limiter {
    gain: f32,
    release: f32,
}

impl Limiter {
    pub fn new(rate: f32) -> Self {
        Self { gain: 1.0, release: (-1.0 / (0.15 * rate)).exp() }
    }

    pub fn set_rate(&mut self, rate: f32) {
        self.release = (-1.0 / (0.15 * rate)).exp();
    }

    pub fn process(&mut self, stereo: &mut [f32]) {
        const CEILING: f32 = 0.97;
        for frame in stereo.chunks_exact_mut(2) {
            let peak = frame[0].abs().max(frame[1].abs());
            let wanted = if peak * self.gain > CEILING { CEILING / peak } else { 1.0 };
            self.gain = if wanted < self.gain {
                wanted
            } else {
                wanted + (self.gain - wanted) * self.release
            };
            frame[0] *= self.gain;
            frame[1] *= self.gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine_gain(gains: [f32; 6], hz: f32) -> f32 {
        let rate = 48000.0;
        let mut eq = Equalizer::new();
        eq.configure(true, &gains, rate);
        let mut buf = vec![0f32; 96000];
        for (i, f) in buf.chunks_exact_mut(2).enumerate() {
            let s = (2.0 * std::f32::consts::PI * hz * i as f32 / rate).sin() * 0.1;
            f[0] = s;
            f[1] = s;
        }
        let rms = |b: &[f32]| (b.iter().map(|x| x * x).sum::<f32>() / b.len() as f32).sqrt();
        let before = rms(&buf[48000..]);
        eq.process(&mut buf);
        20.0 * (rms(&buf[48000..]) / before).log10()
    }

    #[test]
    fn peaking_band_boosts_its_frequency_by_the_set_amount() {
        let g = sine_gain([0.0, 0.0, 0.0, 6.0, 0.0, 0.0], 1000.0);
        assert!((g - 6.0).abs() < 0.3, "got {g}");
    }

    #[test]
    fn low_shelf_leaves_highs_alone() {
        let g = sine_gain([6.0, 0.0, 0.0, 0.0, 0.0, 0.0], 8000.0);
        assert!(g.abs() < 0.3, "got {g}");
    }

    #[test]
    fn limiter_keeps_peaks_below_full_scale() {
        let mut l = Limiter::new(48000.0);
        let mut buf = vec![1.8f32; 4800];
        l.process(&mut buf);
        assert!(buf.iter().all(|s| s.abs() <= 0.971));
    }
}
