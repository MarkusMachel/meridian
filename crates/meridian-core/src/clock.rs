//! Hourly clock with a real (proleptic Gregorian) calendar and rollover signals.
//! The tick count is the only stored state; the date is derived from it with
//! integer math.

/// Days from 1970-01-01 to the start date 1990-01-01.
const START_DAYS_SINCE_1970: i64 = 7305;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Date {
    pub year: i32,
    pub month: u8, // 1..=12
    pub day: u8,   // 1..=31
    pub hour: u8,  // 0..=23
}

/// Rollover signals for the tick that is about to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Signals {
    pub new_day: bool,
    pub new_month: bool,
    pub new_year: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Clock {
    tick: u64,
}

impl Clock {
    pub fn from_tick(tick: u64) -> Clock {
        Clock { tick }
    }
    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn advance(&mut self) {
        self.tick += 1;
    }
    pub fn date(&self) -> Date {
        date_at(self.tick)
    }
    /// Signals for the current tick. Tick 0 is a new day, month and year.
    pub fn signals(&self) -> Signals {
        let d = self.date();
        let new_day = d.hour == 0;
        let new_month = new_day && d.day == 1;
        Signals { new_day, new_month, new_year: new_month && d.month == 1 }
    }
}

pub fn is_leap_year(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn date_at(tick: u64) -> Date {
    let days = (tick / 24) as i64 + START_DAYS_SINCE_1970;
    let hour = (tick % 24) as u8;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u8;
    let year = (if month <= 2 { y + 1 } else { y }) as i32;
    Date { year, month, day, hour }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_1990_01_01() {
        assert_eq!(date_at(0), Date { year: 1990, month: 1, day: 1, hour: 0 });
        assert_eq!(date_at(25), Date { year: 1990, month: 1, day: 2, hour: 1 });
    }

    #[test]
    fn leap_years() {
        assert!(is_leap_year(1992) && is_leap_year(2000));
        assert!(!is_leap_year(1990) && !is_leap_year(1900) && !is_leap_year(2100));
        // 1990 has 365 days, 1991 365, 1992 366.
        let t = |y: i32| {
            let mut c = Clock::default();
            while c.date().year < y {
                c.advance();
            }
            c.tick()
        };
        assert_eq!(t(1991), 365 * 24);
        assert_eq!(t(1992), 730 * 24);
        assert_eq!(t(1993), (730 + 366) * 24);
    }

    #[test]
    fn feb_29_1992_exists() {
        // Day 730 + 31 + 28 = 789 (0-based) is 1992-02-29.
        let d = date_at((730 + 31 + 28) * 24);
        assert_eq!((d.year, d.month, d.day), (1992, 2, 29));
        let d = date_at((730 + 31 + 29) * 24);
        assert_eq!((d.year, d.month, d.day), (1992, 3, 1));
    }

    #[test]
    fn rollover_signals_count_over_a_year() {
        let mut c = Clock::default();
        let (mut days, mut months, mut years) = (0, 0, 0);
        for _ in 0..8760 {
            let s = c.signals();
            days += s.new_day as u32;
            months += s.new_month as u32;
            years += s.new_year as u32;
            c.advance();
        }
        assert_eq!((days, months, years), (365, 12, 1));
        assert!(c.signals().new_year); // tick 8760 is 1991-01-01 00:00
    }
}
