//! Exact rational time, frame rates, and timecode (docs/02 §2).
//!
//! Rules enforced here:
//! - no floats in timing; all arithmetic is exact with i128 intermediates;
//! - `frame_rate` (frames/s) and `frame_duration` (s/frame) are never confused;
//! - timecode is a label: NDF is canonical output, DF is accepted as input at 29.97 / 59.94 only.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Ordering;
use std::fmt;

/// Hard sanity cap on sequence length (docs/02 §2.2).
pub const MAX_SEQUENCE_SECONDS: i64 = 24 * 3600;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimeError {
    #[error("time arithmetic overflow")]
    Overflow,
    #[error("denominator must be positive")]
    ZeroDenominator,
    #[error("time {0} is not on a frame boundary")]
    OffFrameBoundary(Time),
    #[error("invalid timecode: {0}")]
    InvalidTimecode(String),
    #[error("drop-frame timecode is only allowed at 29.97 and 59.94")]
    DropFrameNotAllowed,
    #[error("unsupported frame rate: {0}")]
    UnsupportedFrameRate(String),
    #[error("inexact frame rate {0}: use an exact rational or one of 23.976, 29.97, 59.94")]
    InexactFrameRate(String),
    #[error("negative time not allowed here")]
    Negative,
}

fn gcd(mut a: i128, mut b: i128) -> i128 {
    a = a.abs();
    b = b.abs();
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}

/// Exact rational seconds. Normalized: `den > 0`, `gcd(|num|, den) == 1`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Time {
    num: i64,
    den: u32,
}

impl Time {
    pub const ZERO: Time = Time { num: 0, den: 1 };

    /// Build and normalize. Fails if `den == 0` or the reduced value does not fit.
    pub fn new(num: i64, den: i64) -> Result<Time, TimeError> {
        Self::from_i128(num as i128, den as i128)
    }

    fn from_i128(num: i128, den: i128) -> Result<Time, TimeError> {
        if den == 0 {
            return Err(TimeError::ZeroDenominator);
        }
        let (mut n, mut d) = if den < 0 { (-num, -den) } else { (num, den) };
        let g = gcd(n, d);
        if g > 1 {
            n /= g;
            d /= g;
        }
        if n == 0 {
            d = 1;
        }
        let num = i64::try_from(n).map_err(|_| TimeError::Overflow)?;
        let den = u32::try_from(d).map_err(|_| TimeError::Overflow)?;
        Ok(Time { num, den })
    }

    /// Whole seconds.
    pub fn secs(s: i64) -> Time {
        Time { num: s, den: 1 }
    }

    pub fn num(self) -> i64 {
        self.num
    }
    pub fn den(self) -> u32 {
        self.den
    }
    pub fn is_zero(self) -> bool {
        self.num == 0
    }
    pub fn is_negative(self) -> bool {
        self.num < 0
    }

    pub fn checked_add(self, o: Time) -> Result<Time, TimeError> {
        let n = self.num as i128 * o.den as i128 + o.num as i128 * self.den as i128;
        Self::from_i128(n, self.den as i128 * o.den as i128)
    }
    pub fn checked_sub(self, o: Time) -> Result<Time, TimeError> {
        self.checked_add(Time { num: -o.num, den: o.den })
    }
    /// Multiply by an exact ratio (e.g. a clip speed).
    pub fn checked_mul(self, r: Ratio) -> Result<Time, TimeError> {
        Self::from_i128(self.num as i128 * r.num as i128, self.den as i128 * r.den as i128)
    }
    /// Divide by an exact ratio.
    pub fn checked_div(self, r: Ratio) -> Result<Time, TimeError> {
        Self::from_i128(self.num as i128 * r.den as i128, self.den as i128 * r.num as i128)
    }

    /// Lossy conversion for display and DSP only. Never use for timing decisions.
    pub fn to_f64_lossy(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

impl Ord for Time {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.num as i128 * o.den as i128).cmp(&(o.num as i128 * self.den as i128))
    }
}
impl PartialOrd for Time {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl fmt::Debug for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}s", self.num, self.den)
    }
}
impl fmt::Display for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}s", self.num, self.den)
    }
}

/// Wire form of exact rationals: `{"num": i64, "den": i64}`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RationalWire {
    pub num: i64,
    pub den: i64,
}

impl Serialize for Time {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        RationalWire { num: self.num, den: self.den as i64 }.serialize(s)
    }
}
impl<'de> Deserialize<'de> for Time {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = RationalWire::deserialize(d)?;
        Time::new(w.num, w.den).map_err(serde::de::Error::custom)
    }
}

/// An exact positive-or-zero ratio (speed factors, frame rates). Normalized like `Time`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ratio {
    num: i64,
    den: u32,
}

impl Ratio {
    pub const ONE: Ratio = Ratio { num: 1, den: 1 };
    pub fn new(num: i64, den: i64) -> Result<Ratio, TimeError> {
        let t = Time::new(num, den)?;
        Ok(Ratio { num: t.num, den: t.den })
    }
    pub fn num(self) -> i64 {
        self.num
    }
    pub fn den(self) -> u32 {
        self.den
    }
    pub fn is_positive(self) -> bool {
        self.num > 0
    }
    pub fn to_f64_lossy(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}
impl fmt::Debug for Ratio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}
impl fmt::Display for Ratio {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}
impl Ord for Ratio {
    fn cmp(&self, o: &Self) -> Ordering {
        (self.num as i128 * o.den as i128).cmp(&(o.num as i128 * self.den as i128))
    }
}
impl PartialOrd for Ratio {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Serialize for Ratio {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        RationalWire { num: self.num, den: self.den as i64 }.serialize(s)
    }
}
impl<'de> Deserialize<'de> for Ratio {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = RationalWire::deserialize(d)?;
        Ratio::new(w.num, w.den).map_err(serde::de::Error::custom)
    }
}

/// Rounding mode for time → frame / sample conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Round {
    Floor,
    Ceil,
    Nearest,
    /// Error unless the time is exactly on a boundary.
    Exact,
}

fn div_round(n: i128, d: i128, mode: Round) -> Option<i128> {
    debug_assert!(d > 0);
    let q = n.div_euclid(d);
    let r = n.rem_euclid(d);
    match mode {
        Round::Floor => Some(q),
        Round::Ceil => Some(if r == 0 { q } else { q + 1 }),
        Round::Nearest => Some(if 2 * r >= d { q + 1 } else { q }),
        Round::Exact => (r == 0).then_some(q),
    }
}

/// The eight supported sequence frame rates (docs/02 §2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RationalWire", into = "RationalWire")]
pub enum FrameRate {
    Fps23_976,
    Fps24,
    Fps25,
    Fps29_97,
    Fps30,
    Fps50,
    Fps59_94,
    Fps60,
}

impl TryFrom<RationalWire> for FrameRate {
    type Error = TimeError;
    fn try_from(w: RationalWire) -> Result<Self, TimeError> {
        FrameRate::from_ratio(Ratio::new(w.num, w.den)?)
    }
}
impl From<FrameRate> for RationalWire {
    fn from(f: FrameRate) -> Self {
        let r = f.rate();
        RationalWire { num: r.num, den: r.den as i64 }
    }
}

impl FrameRate {
    pub const ALL: [FrameRate; 8] = [
        FrameRate::Fps23_976,
        FrameRate::Fps24,
        FrameRate::Fps25,
        FrameRate::Fps29_97,
        FrameRate::Fps30,
        FrameRate::Fps50,
        FrameRate::Fps59_94,
        FrameRate::Fps60,
    ];

    /// Exact frames per second.
    pub fn rate(self) -> Ratio {
        let (n, d) = match self {
            FrameRate::Fps23_976 => (24000, 1001),
            FrameRate::Fps24 => (24, 1),
            FrameRate::Fps25 => (25, 1),
            FrameRate::Fps29_97 => (30000, 1001),
            FrameRate::Fps30 => (30, 1),
            FrameRate::Fps50 => (50, 1),
            FrameRate::Fps59_94 => (60000, 1001),
            FrameRate::Fps60 => (60, 1),
        };
        Ratio { num: n, den: d }
    }

    /// Exact seconds per frame (the stored timebase).
    pub fn frame_duration(self) -> Time {
        let r = self.rate();
        Time { num: r.den as i64, den: r.num as u32 }
    }

    /// Integer frames per timecode second.
    pub fn nominal(self) -> i64 {
        match self {
            FrameRate::Fps23_976 | FrameRate::Fps24 => 24,
            FrameRate::Fps25 => 25,
            FrameRate::Fps29_97 | FrameRate::Fps30 => 30,
            FrameRate::Fps50 => 50,
            FrameRate::Fps59_94 | FrameRate::Fps60 => 60,
        }
    }

    /// Labels dropped per minute in drop-frame timecode, if DF is allowed.
    pub fn drop_frames(self) -> Option<i64> {
        match self {
            FrameRate::Fps29_97 => Some(2),
            FrameRate::Fps59_94 => Some(4),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FrameRate::Fps23_976 => "23.976",
            FrameRate::Fps24 => "24",
            FrameRate::Fps25 => "25",
            FrameRate::Fps29_97 => "29.97",
            FrameRate::Fps30 => "30",
            FrameRate::Fps50 => "50",
            FrameRate::Fps59_94 => "59.94",
            FrameRate::Fps60 => "60",
        }
    }

    pub fn from_ratio(r: Ratio) -> Result<FrameRate, TimeError> {
        FrameRate::ALL
            .into_iter()
            .find(|f| f.rate() == r)
            .ok_or_else(|| TimeError::UnsupportedFrameRate(r.to_string()))
    }

    /// Parse `"30000/1001"`, `"30/1"`, `"30"`, or the decimal aliases `23.976`, `29.97`, `59.94`.
    pub fn parse(s: &str) -> Result<FrameRate, TimeError> {
        let s = s.trim();
        match s {
            "23.976" => return Ok(FrameRate::Fps23_976),
            "29.97" => return Ok(FrameRate::Fps29_97),
            "59.94" => return Ok(FrameRate::Fps59_94),
            _ => {}
        }
        if let Some((n, d)) = s.split_once('/') {
            let n: i64 = n.trim().parse().map_err(|_| TimeError::UnsupportedFrameRate(s.into()))?;
            let d: i64 = d.trim().parse().map_err(|_| TimeError::UnsupportedFrameRate(s.into()))?;
            return FrameRate::from_ratio(Ratio::new(n, d)?);
        }
        if s.contains('.') {
            return Err(TimeError::InexactFrameRate(s.into()));
        }
        let n: i64 = s.parse().map_err(|_| TimeError::UnsupportedFrameRate(s.into()))?;
        FrameRate::from_ratio(Ratio::new(n, 1)?)
    }

    /// Wire string form, e.g. `"30000/1001"`.
    pub fn wire(self) -> String {
        self.rate().to_string()
    }

    /// Frame index containing / nearest to `t`.
    pub fn frame_at(self, t: Time, mode: Round) -> Result<i64, TimeError> {
        let r = self.rate();
        // frames = t * rate = (t.num * r.num) / (t.den * r.den)
        let n = t.num as i128 * r.num as i128;
        let d = t.den as i128 * r.den as i128;
        let q = div_round(n, d, mode).ok_or(TimeError::OffFrameBoundary(t))?;
        i64::try_from(q).map_err(|_| TimeError::Overflow)
    }

    pub fn time_of_frame(self, frame: i64) -> Time {
        let r = self.rate();
        // exact by construction; cannot overflow for frames within the 24 h cap
        Time::from_i128(frame as i128 * r.den as i128, r.num as i128).unwrap_or(Time::ZERO)
    }

    pub fn is_on_boundary(self, t: Time) -> bool {
        self.frame_at(t, Round::Exact).is_ok()
    }

    /// Maximum frame count for the 24 h sequence cap.
    pub fn max_frames(self) -> i64 {
        // floor(24h * rate)
        self.frame_at(Time::secs(MAX_SEQUENCE_SECONDS), Round::Floor).unwrap_or(0)
    }

    // ---- timecode -------------------------------------------------------------------------

    /// Non-drop-frame label `HH:MM:SS:FF` (canonical output).
    pub fn format_ndf(self, frame: i64) -> String {
        let nom = self.nominal();
        let neg = frame < 0;
        let f = frame.abs();
        let ff = f % nom;
        let total_s = f / nom;
        let s = total_s % 60;
        let m = (total_s / 60) % 60;
        let h = total_s / 3600;
        format!("{}{:02}:{:02}:{:02}:{:02}", if neg { "-" } else { "" }, h, m, s, ff)
    }

    /// Drop-frame label `HH:MM:SS;FF`. Errors at non-DF rates.
    pub fn format_df(self, frame: i64) -> Result<String, TimeError> {
        let drop = self.drop_frames().ok_or(TimeError::DropFrameNotAllowed)?;
        if frame < 0 {
            return Err(TimeError::Negative);
        }
        let nom = self.nominal();
        let fpm = nom * 60 - drop;
        let fp10m = nom * 600 - drop * 9;
        let d = frame / fp10m;
        let m = frame % fp10m;
        let label_frames = if m > drop {
            frame + drop * 9 * d + drop * ((m - drop) / fpm)
        } else {
            frame + drop * 9 * d
        };
        let ndf = self.format_ndf(label_frames);
        // replace the last ':' with ';'
        let idx = ndf.rfind(':').unwrap_or(0);
        Ok(format!("{};{}", &ndf[..idx], &ndf[idx + 1..]))
    }

    /// Parse NDF `HH:MM:SS:FF` or DF `HH:MM:SS;FF` into a frame index.
    pub fn parse_timecode(self, s: &str) -> Result<i64, TimeError> {
        let s = s.trim();
        let bad = || TimeError::InvalidTimecode(s.to_string());
        let (head, ff_str, df) = match (s.rfind(';'), s.rfind(':')) {
            (Some(i), _) => (&s[..i], &s[i + 1..], true),
            (None, Some(i)) => (&s[..i], &s[i + 1..], false),
            _ => return Err(bad()),
        };
        let parts: Vec<&str> = head.split(':').collect();
        if parts.len() != 3 {
            return Err(bad());
        }
        let num = |x: &str| -> Result<i64, TimeError> {
            if x.is_empty() || x.len() > 2 || !x.bytes().all(|b| b.is_ascii_digit()) {
                return Err(bad());
            }
            x.parse::<i64>().map_err(|_| bad())
        };
        let (h, m, sec, ff) = (num(parts[0])?, num(parts[1])?, num(parts[2])?, num(ff_str)?);
        let nom = self.nominal();
        if m >= 60 || sec >= 60 || ff >= nom || h >= 24 {
            return Err(bad());
        }
        let label = (h * 3600 + m * 60 + sec) * nom + ff;
        if !df {
            return Ok(label);
        }
        let drop = self.drop_frames().ok_or(TimeError::DropFrameNotAllowed)?;
        if sec == 0 && ff < drop && m % 10 != 0 {
            return Err(bad());
        }
        let total_minutes = h * 60 + m;
        Ok(label - drop * (total_minutes - total_minutes / 10))
    }

    /// Parse a timecode into an exact `Time` at this rate.
    pub fn timecode_to_time(self, s: &str) -> Result<Time, TimeError> {
        Ok(self.time_of_frame(self.parse_timecode(s)?))
    }

    /// Format an exact on-boundary time as NDF; off-boundary times are floored.
    pub fn time_to_timecode(self, t: Time) -> String {
        self.format_ndf(self.frame_at(t, Round::Floor).unwrap_or(0))
    }
}

/// Sample index of `t` at an integer sample rate.
pub fn sample_index(t: Time, sample_rate: u32, mode: Round) -> Result<i64, TimeError> {
    let n = t.num as i128 * sample_rate as i128;
    let q = div_round(n, t.den as i128, mode).ok_or(TimeError::OffFrameBoundary(t))?;
    i64::try_from(q).map_err(|_| TimeError::Overflow)
}

/// Time of an integer sample index.
pub fn time_of_sample(index: i64, sample_rate: u32) -> Result<Time, TimeError> {
    Time::new(index, sample_rate as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(n: i64, d: i64) -> Time {
        Time::new(n, d).expect("valid")
    }

    #[test]
    fn normalizes() {
        assert_eq!(t(2, 4), t(1, 2));
        assert_eq!(t(-3, -6), t(1, 2));
        assert_eq!(t(3, -6), t(-1, 2));
        assert_eq!(t(0, 7), Time::ZERO);
        assert!(Time::new(1, 0).is_err());
    }

    #[test]
    fn arithmetic_exact() {
        let f = FrameRate::Fps29_97.frame_duration();
        let mut acc = Time::ZERO;
        for _ in 0..30000 {
            acc = acc.checked_add(f).expect("add");
        }
        assert_eq!(acc, Time::secs(1001));
        assert!(t(1, 3) < t(1, 2));
        assert_eq!(t(1, 2).checked_mul(Ratio::new(2, 1).expect("r")).expect("m"), Time::secs(1));
    }

    #[test]
    fn overflow_detected() {
        let big = Time::new(i64::MAX, 1).expect("t");
        assert_eq!(big.checked_add(big), Err(TimeError::Overflow));
    }

    #[test]
    fn frame_rate_parsing() {
        assert_eq!(FrameRate::parse("29.97"), Ok(FrameRate::Fps29_97));
        assert_eq!(FrameRate::parse("30000/1001"), Ok(FrameRate::Fps29_97));
        assert_eq!(FrameRate::parse("60000/1001"), Ok(FrameRate::Fps59_94));
        assert_eq!(FrameRate::parse("25"), Ok(FrameRate::Fps25));
        assert!(matches!(FrameRate::parse("29.976"), Err(TimeError::InexactFrameRate(_))));
        assert!(matches!(FrameRate::parse("30.0"), Err(TimeError::InexactFrameRate(_))));
        assert!(matches!(FrameRate::parse("48"), Err(TimeError::UnsupportedFrameRate(_))));
        for f in FrameRate::ALL {
            let r = f.rate();
            let d = f.frame_duration();
            assert_eq!(
                (r.num() as i128) * (d.num() as i128),
                (r.den() as i128) * (d.den() as i128),
                "rate x duration == 1 for {f:?}"
            );
            assert_eq!(FrameRate::parse(&f.wire()), Ok(f));
        }
    }

    #[test]
    fn frame_at_rounding() {
        let f = FrameRate::Fps29_97;
        let b = f.time_of_frame(10);
        let tick = t(1, 1_000_000);
        assert_eq!(f.frame_at(b, Round::Exact), Ok(10));
        assert_eq!(f.frame_at(b.checked_sub(tick).expect("s"), Round::Floor), Ok(9));
        assert_eq!(f.frame_at(b.checked_sub(tick).expect("s"), Round::Ceil), Ok(10));
        assert_eq!(f.frame_at(b.checked_add(tick).expect("a"), Round::Floor), Ok(10));
        assert_eq!(f.frame_at(b.checked_add(tick).expect("a"), Round::Ceil), Ok(11));
        assert_eq!(f.frame_at(b.checked_add(tick).expect("a"), Round::Nearest), Ok(10));
        assert!(f.frame_at(b.checked_add(tick).expect("a"), Round::Exact).is_err());
        // 1 s is not a frame boundary at 29.97
        assert!(!f.is_on_boundary(Time::secs(1)));
        assert!(FrameRate::Fps30.is_on_boundary(Time::secs(1)));
    }

    #[test]
    fn ndf_vectors() {
        let f = FrameRate::Fps29_97;
        assert_eq!(f.format_ndf(108_000), "01:00:00:00");
        assert_eq!(f.time_of_frame(108_000), t(3_603_600, 1000)); // 3603.6 s
        assert_eq!(f.parse_timecode("01:00:00:00"), Ok(108_000));
    }

    #[test]
    fn df_vectors_2997() {
        let f = FrameRate::Fps29_97;
        for (frame, label) in [
            (1799, "00:00:59;29"),
            (1800, "00:01:00;02"),
            (17_982, "00:10:00;00"),
            (107_892, "01:00:00;00"),
        ] {
            assert_eq!(f.format_df(frame).expect("df"), label);
            assert_eq!(f.parse_timecode(label), Ok(frame));
        }
    }

    #[test]
    fn df_vectors_5994() {
        let f = FrameRate::Fps59_94;
        assert_eq!(f.format_df(3599).expect("df"), "00:00:59;59");
        assert_eq!(f.format_df(3600).expect("df"), "00:01:00;04");
        assert_eq!(f.parse_timecode("00:01:00;04"), Ok(3600));
    }

    #[test]
    fn timecode_rejections() {
        let f = FrameRate::Fps29_97;
        for bad in ["00:01:00;00", "00:01:00;01", "00:00:00:30", "00:60:00:00", "00:00:60:00", "24:00:00:00", "1:2:3", "aa:00:00:00"] {
            assert!(f.parse_timecode(bad).is_err(), "{bad} should be rejected");
        }
        assert!(f.parse_timecode("00:10:00;00").is_ok());
        let g = FrameRate::Fps59_94;
        for ff in 0..4 {
            assert!(g.parse_timecode(&format!("00:01:00;{ff:02}")).is_err());
        }
        assert_eq!(FrameRate::Fps25.parse_timecode("00:00:01;00"), Err(TimeError::DropFrameNotAllowed));
        assert_eq!(FrameRate::Fps30.format_df(1), Err(TimeError::DropFrameNotAllowed));
    }

    /// Exhaustive 24 h round trip (docs/08 §6). Runs in a few seconds with the test profile.
    #[test]
    fn exhaustive_round_trip_24h() {
        for f in FrameRate::ALL {
            let max = f.max_frames();
            let step = if f.drop_frames().is_some() { 1 } else { 7 };
            let mut frame = 0;
            while frame < max {
                let ndf = f.format_ndf(frame);
                assert_eq!(f.parse_timecode(&ndf), Ok(frame), "{f:?} ndf {ndf}");
                if f.drop_frames().is_some() {
                    let df = f.format_df(frame).expect("df");
                    assert_eq!(f.parse_timecode(&df), Ok(frame), "{f:?} df {df}");
                }
                frame += step;
            }
        }
    }

    #[test]
    fn sample_mapping() {
        // one 29.97 frame = 1601.6 samples at 48 kHz
        let d = FrameRate::Fps29_97.frame_duration();
        assert_eq!(sample_index(d, 48_000, Round::Floor), Ok(1601));
        assert!(sample_index(d, 48_000, Round::Exact).is_err());
        let five = FrameRate::Fps29_97.time_of_frame(5);
        assert_eq!(sample_index(five, 48_000, Round::Exact), Ok(8008));
        for f in FrameRate::ALL {
            for sr in [44_100u32, 48_000, 96_000] {
                let s1 = f.time_of_frame(1001);
                let idx = sample_index(s1, sr, Round::Nearest).expect("idx");
                let back = time_of_sample(idx, sr).expect("t");
                let err = back.checked_sub(s1).expect("s");
                assert!(err.to_f64_lossy().abs() <= 0.5 / sr as f64 + 1e-12);
            }
        }
    }

    #[test]
    fn serde_forms() {
        let v = serde_json::to_string(&t(1001, 30000)).expect("ser");
        assert_eq!(v, r#"{"num":1001,"den":30000}"#);
        let back: Time = serde_json::from_str(r#"{"num":2,"den":60}"#).expect("de");
        assert_eq!(back, t(1, 30));
        let fr: FrameRate = serde_json::from_str(r#"{"num":30000,"den":1001}"#).expect("fr");
        assert_eq!(fr, FrameRate::Fps29_97);
        assert!(serde_json::from_str::<FrameRate>(r#"{"num":48,"den":1}"#).is_err());
    }
}
