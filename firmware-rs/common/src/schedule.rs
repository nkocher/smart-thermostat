use chrono::{DateTime, Datelike, Duration, FixedOffset, TimeZone, Timelike, Weekday};
use serde::{Deserialize, Serialize};

use crate::types::ThermostatMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum DayOfWeek {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl DayOfWeek {
    pub fn index(self) -> usize {
        match self {
            Self::Mon => 0,
            Self::Tue => 1,
            Self::Wed => 2,
            Self::Thu => 3,
            Self::Fri => 4,
            Self::Sat => 5,
            Self::Sun => 6,
        }
    }

    pub fn from_index(index: usize) -> Self {
        match index % 7 {
            0 => Self::Mon,
            1 => Self::Tue,
            2 => Self::Wed,
            3 => Self::Thu,
            4 => Self::Fri,
            5 => Self::Sat,
            _ => Self::Sun,
        }
    }

    pub fn from_chrono(weekday: Weekday) -> Self {
        match weekday {
            Weekday::Mon => Self::Mon,
            Weekday::Tue => Self::Tue,
            Weekday::Wed => Self::Wed,
            Weekday::Thu => Self::Thu,
            Weekday::Fri => Self::Fri,
            Weekday::Sat => Self::Sat,
            Weekday::Sun => Self::Sun,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScheduleEntry {
    pub day: DayOfWeek,
    #[serde(rename = "startMinutes")]
    pub start_minutes: u16,
    pub mode: ThermostatMode,
    #[serde(rename = "targetTemp")]
    pub target_temp_f: f32,
}

impl ScheduleEntry {
    pub fn validate(&self) -> bool {
        self.start_minutes < 24 * 60
            && self.target_temp_f.is_finite()
            && (60.0..=84.0).contains(&self.target_temp_f)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Schedule {
    pub enabled: bool,
    pub entries: Vec<ScheduleEntry>,
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            enabled: false,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScheduleAction {
    pub mode: ThermostatMode,
    pub target_temp_f: f32,
}

impl Schedule {
    pub fn normalize(&mut self) {
        self.entries.retain(ScheduleEntry::validate);
        self.entries
            .sort_by_key(|entry| (entry.day.index(), entry.start_minutes));
    }

    pub fn current_action(&self, now: DateTime<FixedOffset>) -> Option<ScheduleAction> {
        if !self.enabled || self.entries.is_empty() {
            return None;
        }

        let day = DayOfWeek::from_chrono(now.weekday());
        let current_minutes = now.hour() as u16 * 60 + now.minute() as u16;

        // Current day, last entry <= now.
        let mut best: Option<&ScheduleEntry> = self
            .entries
            .iter()
            .filter(|entry| entry.day == day && entry.start_minutes <= current_minutes)
            .max_by_key(|entry| entry.start_minutes);

        // Wrap to previous days until we find one.
        if best.is_none() {
            for i in 1..=7 {
                let candidate_day = DayOfWeek::from_index((day.index() + 7 - i) % 7);
                best = self
                    .entries
                    .iter()
                    .filter(|entry| entry.day == candidate_day)
                    .max_by_key(|entry| entry.start_minutes);

                if best.is_some() {
                    break;
                }
            }
        }

        best.map(|entry| ScheduleAction {
            mode: entry.mode,
            target_temp_f: entry.target_temp_f,
        })
    }

    pub fn next_event_epoch(&self, now: DateTime<FixedOffset>) -> Option<i64> {
        if !self.enabled || self.entries.is_empty() {
            return None;
        }

        let now_day = DayOfWeek::from_chrono(now.weekday());
        let now_minute = now.hour() as i64 * 60 + now.minute() as i64;

        let mut best: Option<DateTime<FixedOffset>> = None;

        for day_offset in 0..7i64 {
            let day = DayOfWeek::from_index((now_day.index() + day_offset as usize) % 7);
            for entry in self.entries.iter().filter(|entry| entry.day == day) {
                let candidate_minutes = entry.start_minutes as i64;
                if day_offset == 0 && candidate_minutes <= now_minute {
                    continue;
                }

                let date = now.date_naive() + Duration::days(day_offset);
                let hour = (entry.start_minutes / 60) as u32;
                let minute = (entry.start_minutes % 60) as u32;

                let Some(naive) = date.and_hms_opt(hour, minute, 0) else {
                    continue;
                };

                let Some(candidate) = now.offset().from_local_datetime(&naive).single() else {
                    continue;
                };

                if best.map(|current| candidate < current).unwrap_or(true) {
                    best = Some(candidate);
                }
            }
        }

        best.map(|dt| dt.timestamp())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_time(day: u32, hour: u32, minute: u32) -> DateTime<FixedOffset> {
        FixedOffset::west_opt(8 * 3600)
            .unwrap()
            .with_ymd_and_hms(2026, 1, day, hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn wraps_schedule_to_previous_day() {
        let mut schedule = Schedule {
            enabled: true,
            entries: vec![ScheduleEntry {
                day: DayOfWeek::Sun,
                start_minutes: 23 * 60,
                mode: ThermostatMode::Heat,
                target_temp_f: 69.0,
            }],
        };
        schedule.normalize();

        // Monday 08:00 should still be affected by Sunday 23:00 program.
        let now = fixed_time(5, 8, 0); // Jan 5, 2026 is Monday.
        let action = schedule.current_action(now).unwrap();

        assert_eq!(action.mode, ThermostatMode::Heat);
        assert_eq!(action.target_temp_f, 69.0);
    }

    #[test]
    fn finds_next_event_in_current_week() {
        let mut schedule = Schedule {
            enabled: true,
            entries: vec![
                ScheduleEntry {
                    day: DayOfWeek::Mon,
                    start_minutes: 9 * 60,
                    mode: ThermostatMode::Heat,
                    target_temp_f: 71.0,
                },
                ScheduleEntry {
                    day: DayOfWeek::Mon,
                    start_minutes: 18 * 60,
                    mode: ThermostatMode::Off,
                    target_temp_f: 68.0,
                },
            ],
        };
        schedule.normalize();

        let now = fixed_time(5, 9, 1);
        let next = schedule.next_event_epoch(now).unwrap();
        let expected = fixed_time(5, 18, 0).timestamp();

        assert_eq!(next, expected);
    }
}
