//! Numbers measured from the original program. MECHANICS.md lists where each came from.

pub const SEEK_STEP_MS: u64 = 5_000;
pub const LEFT_DEFAULT: f32 = 303.0;
pub const RIGHT_WIDTH: f32 = 303.0;
pub const CROSSFADE_MAX_S: u32 = 12;
pub const PREVIOUS_RESTART_MS: u64 = 3_000;
/// Each device reports its own count (capped at `VOLUME_STEPS_CAP`); this client's isn't in the code.
pub const VOLUME_STEPS: u32 = 10;
pub const VOLUME_STEPS_CAP: u32 = 0x200;

pub const LOUDNESS_TARGET_DB: f32 = -14.0;
pub const GAIN_MIN_DB: f32 = -40.0;
pub const GAIN_MAX_DB: f32 = 42.0;
pub const QUIET_OFFSET_DB: f32 = -9.0;
pub const LOUD_OFFSET_DB: f32 = 3.0;
pub const PEAK_CEILING_LOSSY_DB: f32 = -1.0;
pub const PEAK_CEILING_LOSSLESS_DB: f32 = 0.0;

pub const EQ_RANGE_DB: f32 = 12.0;
pub const EQ_Q: f32 = 1.0;

pub const EQ_BANDS: [EqBand; 6] = [
    EqBand { kind: EqKind::LowShelf, hz: 60.0, label: "60 Hz" },
    EqBand { kind: EqKind::Peaking, hz: 150.0, label: "150 Hz" },
    EqBand { kind: EqKind::Peaking, hz: 400.0, label: "400 Hz" },
    EqBand { kind: EqKind::Peaking, hz: 1000.0, label: "1 kHz" },
    EqBand { kind: EqKind::Peaking, hz: 2400.0, label: "2,4 kHz" },
    EqBand { kind: EqKind::HighShelf, hz: 15000.0, label: "15 kHz" },
];

#[derive(Clone, Copy)]
pub struct EqBand {
    pub kind: EqKind,
    pub hz: f32,
    pub label: &'static str,
}

#[derive(Clone, Copy, PartialEq)]
pub enum EqKind {
    LowShelf,
    Peaking,
    HighShelf,
}

pub struct EqPreset {
    pub vi: &'static str,
    pub en: &'static str,
    pub gains: [f32; 6],
}

macro_rules! preset {
    ($vi:expr, $en:expr, $g:expr) => {
        EqPreset { vi: $vi, en: $en, gains: $g }
    };
}

pub const EQ_PRESETS: &[EqPreset] = &[
    preset!("Phẳng", "Flat", [0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    preset!("Acoustic", "Acoustic", [4.9, 3.95, 2.15, 1.75, 3.5, 2.15]),
    preset!("Tăng bass", "Bass booster", [4.25, 3.5, 1.25, 0.0, 0.0, 0.0]),
    preset!("Giảm bass", "Bass reducer", [-4.25, -3.5, -1.25, 0.0, 0.0, 0.0]),
    preset!("Cổ điển", "Classical", [3.75, 3.0, -1.5, -1.5, 0.0, 3.75]),
    preset!("Dance", "Dance", [6.55, 4.99, 1.92, 3.65, 5.15, 0.0]),
    preset!("Sâu lắng", "Deep", [3.55, 1.75, 2.85, 2.5, 1.45, -2.15]),
    preset!("Điện tử", "Electronic", [3.8, 1.2, -2.15, 2.25, 0.85, 4.8]),
    preset!("Hip-Hop", "Hip-Hop", [4.25, 1.5, -1.0, -1.0, 1.5, 3.0]),
    preset!("Jazz", "Jazz", [3.0, 1.5, -1.5, -1.5, 0.0, 3.75]),
    preset!("Latin", "Latin", [3.0, 0.0, -1.5, -1.5, -1.5, 4.5]),
    preset!("Âm lượng lớn", "Loudness", [4.0, 0.0, -2.0, 0.0, -1.0, 1.0]),
    preset!("Lounge", "Lounge", [-1.5, -0.5, 4.0, 2.5, 0.0, 1.0]),
    preset!("Piano", "Piano", [2.0, 0.0, 3.0, 1.5, 3.5, 3.5]),
    preset!("Pop", "Pop", [-1.0, 0.0, 4.0, 4.0, 2.0, -1.5]),
    preset!("R&B", "R&B", [6.92, 5.65, -2.19, -1.5, 2.32, 3.75]),
    preset!("Rock", "Rock", [4.0, 3.0, -0.5, -1.0, 0.5, 4.5]),
    preset!("Loa nhỏ", "Small speakers", [4.25, 3.5, 1.25, 0.0, -1.25, -4.25]),
    preset!("Giọng nói", "Spoken word", [-0.47, 0.0, 3.46, 4.61, 4.84, 0.0]),
    preset!("Tăng treble", "Treble booster", [0.0, 0.0, 0.0, 1.25, 2.5, 5.5]),
    preset!("Giảm treble", "Treble reducer", [0.0, 0.0, 0.0, -1.25, -2.5, -5.5]),
    preset!("Tăng giọng hát", "Vocal booster", [-3.0, -3.0, 3.75, 3.75, 3.0, -1.5]),
];

/// How one song hands over to the next (Spotify.dll 0x180d56f4c).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Transition {
    /// A short stop between songs.
    None,
    Gapless,
    Crossfade,
    AutoMix,
}

/// What one press of a volume key moves, as a share of full volume (Spotify.dll 0x180eedd98).
pub fn volume_key_step(steps: u32) -> f32 {
    (0xffff / steps.clamp(1, VOLUME_STEPS_CAP)) as f32 / 65535.0
}

/// The gain for a song, following Spotify.dll 0x180469728. Local files have no server figures, so
/// loudness and peak come from tags or from measuring the file.
pub fn normalization_gain_db(loudness_db: Option<f32>, peak_db: Option<f32>, level_offset_db: f32, loud_level: bool, lossless: bool) -> f32 {
    let base = loudness_db.map_or(0.0, |l| LOUDNESS_TARGET_DB - l);
    if !(GAIN_MIN_DB..=GAIN_MAX_DB).contains(&base) {
        return 0.0;
    }
    let gain = base + level_offset_db;
    // Turning a song down never clips; the Loud level leaves boosts to the limiter.
    let Some(peak) = peak_db.filter(|_| gain > 0.0 && !loud_level) else { return gain };
    let ceiling = if lossless { PEAK_CEILING_LOSSLESS_DB } else { PEAK_CEILING_LOSSY_DB };
    if peak + gain <= ceiling {
        gain
    } else if peak < ceiling {
        ceiling - peak
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain(loudness: Option<f32>, peak: Option<f32>, offset: f32, loud: bool, lossless: bool) -> f32 {
        normalization_gain_db(loudness, peak, offset, loud, lossless)
    }

    #[test]
    fn gain_moves_track_to_target() {
        assert!((gain(Some(-20.0), None, 0.0, false, false) - 6.0).abs() < 1e-3);
        assert!((gain(Some(-8.0), None, 0.0, false, false) + 6.0).abs() < 1e-3);
    }

    #[test]
    fn levels_shift_the_target() {
        assert!((gain(Some(-14.0), None, QUIET_OFFSET_DB, false, false) + 9.0).abs() < 1e-3);
        assert!((gain(Some(-14.0), None, LOUD_OFFSET_DB, true, false) - 3.0).abs() < 1e-3);
    }

    #[test]
    fn nonsense_loudness_is_ignored() {
        assert_eq!(gain(Some(-120.0), None, 0.0, false, false), 0.0);
        assert_eq!(gain(None, None, 0.0, false, false), 0.0);
    }

    #[test]
    fn boosts_stop_at_the_peak_ceiling() {
        assert!((gain(Some(-20.0), Some(-3.0), 0.0, false, false) - 2.0).abs() < 1e-3);
        assert!((gain(Some(-20.0), Some(-3.0), 0.0, false, true) - 3.0).abs() < 1e-3);
        assert!((gain(Some(-20.0), Some(-10.0), 0.0, false, false) - 6.0).abs() < 1e-3);
    }

    #[test]
    fn a_song_already_over_the_ceiling_gets_no_boost() {
        assert_eq!(gain(Some(-20.0), Some(-0.5), 0.0, false, false), 0.0);
    }

    #[test]
    fn loud_level_and_cuts_ignore_the_ceiling() {
        assert!((gain(Some(-20.0), Some(-3.0), LOUD_OFFSET_DB, true, false) - 9.0).abs() < 1e-3);
        assert!((gain(Some(-8.0), Some(-0.1), 0.0, false, false) + 6.0).abs() < 1e-3);
    }

    #[test]
    fn volume_step_divides_the_range_and_caps_the_count() {
        assert!((volume_key_step(10) - 0.1).abs() < 1e-3);
        assert_eq!(volume_key_step(100_000), volume_key_step(512));
    }
}
