use super::super::models::Activity;
use chrono::{Datelike, NaiveDate};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

#[derive(Serialize)]
pub struct Summary {
    pub total_activities: usize,
    pub total_distance_km: f64,
    pub total_moving_time_hours: f64,
    pub total_elevation_m: f64,
    pub total_calories: f64,
    pub avg_distance_km: f64,
    pub avg_moving_time_minutes: f64,
    pub avg_speed_kmh: f64,
    pub avg_heart_rate: Option<f64>,
    pub avg_watts: Option<f64>,
    pub longest_ride_km: f64,
    pub max_elevation_gain_m: f64,
    pub max_speed_kmh: f64,
    pub gear_breakdown: Vec<GearBreakdown>,
}

#[derive(Serialize)]
pub struct GearBreakdown {
    pub gear: String,
    pub count: usize,
    pub distance_km: f64,
}

pub fn compute_summary(activities: &[&Activity]) -> Summary {
    let n = activities.len();
    if n == 0 {
        return Summary {
            total_activities: 0,
            total_distance_km: 0.0,
            total_moving_time_hours: 0.0,
            total_elevation_m: 0.0,
            total_calories: 0.0,
            avg_distance_km: 0.0,
            avg_moving_time_minutes: 0.0,
            avg_speed_kmh: 0.0,
            avg_heart_rate: None,
            avg_watts: None,
            longest_ride_km: 0.0,
            max_elevation_gain_m: 0.0,
            max_speed_kmh: 0.0,
            gear_breakdown: vec![],
        };
    }

    let total_dist: f64 = activities.iter().map(|a| a.distance_meters).sum();
    let total_time: f64 = activities.iter().map(|a| a.moving_time).sum();
    let total_elev: f64 = activities
        .iter()
        .filter_map(|a| a.elevation_gain)
        .sum();
    let total_cal: f64 = activities.iter().filter_map(|a| a.calories).sum();
    let max_dist = activities
        .iter()
        .map(|a| a.distance_meters)
        .fold(0.0f64, f64::max);
    let max_elev = activities
        .iter()
        .filter_map(|a| a.elevation_gain)
        .fold(0.0f64, f64::max);
    let max_speed = activities
        .iter()
        .filter_map(|a| a.max_speed)
        .fold(0.0f64, f64::max);

    let hr_vals: Vec<f64> = activities.iter().filter_map(|a| a.average_heart_rate).collect();
    let avg_hr = if hr_vals.is_empty() {
        None
    } else {
        Some(hr_vals.iter().sum::<f64>() / hr_vals.len() as f64)
    };

    let watt_vals: Vec<f64> = activities.iter().filter_map(|a| a.average_watts).collect();
    let avg_watts = if watt_vals.is_empty() {
        None
    } else {
        Some(watt_vals.iter().sum::<f64>() / watt_vals.len() as f64)
    };

    let mut gear_map: HashMap<String, (usize, f64)> = HashMap::new();
    for a in activities {
        let name = a
            .gear_name
            .clone()
            .unwrap_or_else(|| "(none)".to_string());
        let entry = gear_map.entry(name).or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += a.distance_meters;
    }
    let mut gear_breakdown: Vec<GearBreakdown> = gear_map
        .into_iter()
        .map(|(gear, (count, dist))| GearBreakdown {
            gear,
            count,
            distance_km: dist / 1000.0,
        })
        .collect();
    gear_breakdown.sort_by_key(|a| std::cmp::Reverse(a.count));

    let nf = n as f64;
    let avg_speed = if total_time > 0.0 {
        (total_dist / total_time) * 3.6
    } else {
        0.0
    };

    Summary {
        total_activities: n,
        total_distance_km: total_dist / 1000.0,
        total_moving_time_hours: total_time / 3600.0,
        total_elevation_m: total_elev,
        total_calories: total_cal,
        avg_distance_km: total_dist / 1000.0 / nf,
        avg_moving_time_minutes: total_time / 60.0 / nf,
        avg_speed_kmh: avg_speed,
        avg_heart_rate: avg_hr,
        avg_watts,
        longest_ride_km: max_dist / 1000.0,
        max_elevation_gain_m: max_elev,
        max_speed_kmh: max_speed * 3.6,
        gear_breakdown,
    }
}

#[derive(Serialize)]
pub struct YearStats {
    pub year: i32,
    pub total_activities: usize,
    pub total_distance_km: f64,
    pub total_moving_time_hours: f64,
    pub total_elevation_m: f64,
    pub total_calories: f64,
    pub avg_distance_km: f64,
    pub avg_speed_kmh: f64,
    pub avg_heart_rate: Option<f64>,
    pub avg_watts: Option<f64>,
    pub longest_ride_km: f64,
    pub ride_days: usize,
}

pub fn compute_yearly(activities: &[&Activity]) -> Vec<YearStats> {
    let mut by_year: BTreeMap<i32, Vec<&Activity>> = BTreeMap::new();
    for a in activities {
        by_year.entry(a.date.date().year()).or_default().push(a);
    }

    by_year
        .into_iter()
        .map(|(year, acts)| {
            let total_dist: f64 = acts.iter().map(|a| a.distance_meters).sum();
            let total_time: f64 = acts.iter().map(|a| a.moving_time).sum();
            let total_elev: f64 = acts.iter().filter_map(|a| a.elevation_gain).sum();
            let total_cal: f64 = acts.iter().filter_map(|a| a.calories).sum();
            let max_dist = acts
                .iter()
                .map(|a| a.distance_meters)
                .fold(0.0f64, f64::max);

            let hr_vals: Vec<f64> = acts.iter().filter_map(|a| a.average_heart_rate).collect();
            let avg_hr = if hr_vals.is_empty() {
                None
            } else {
                Some(hr_vals.iter().sum::<f64>() / hr_vals.len() as f64)
            };

            let watt_vals: Vec<f64> = acts.iter().filter_map(|a| a.average_watts).collect();
            let avg_watts = if watt_vals.is_empty() {
                None
            } else {
                Some(watt_vals.iter().sum::<f64>() / watt_vals.len() as f64)
            };

            let mut unique_days = acts
                .iter()
                .map(|a| a.date.date())
                .collect::<Vec<_>>();
            unique_days.sort();
            unique_days.dedup();

            let nf = acts.len() as f64;
            let avg_speed = if total_time > 0.0 {
                (total_dist / total_time) * 3.6
            } else {
                0.0
            };

            YearStats {
                year,
                total_activities: acts.len(),
                total_distance_km: total_dist / 1000.0,
                total_moving_time_hours: total_time / 3600.0,
                total_elevation_m: total_elev,
                total_calories: total_cal,
                avg_distance_km: total_dist / 1000.0 / nf,
                avg_speed_kmh: avg_speed,
                avg_heart_rate: avg_hr,
                avg_watts,
                longest_ride_km: max_dist / 1000.0,
                ride_days: unique_days.len(),
            }
        })
        .collect()
}

#[derive(Serialize)]
pub struct MonthStats {
    pub year: i32,
    pub month: u32,
    pub total_activities: usize,
    pub total_distance_km: f64,
    pub total_elevation_m: f64,
    pub total_moving_time_hours: f64,
    pub avg_speed_kmh: f64,
    pub ride_days: usize,
}

pub fn compute_monthly(activities: &[&Activity], year: Option<i32>) -> Vec<MonthStats> {
    let filtered: Vec<&&Activity> = if let Some(y) = year {
        activities.iter().filter(|a| a.date.date().year() == y).collect()
    } else {
        activities.iter().collect()
    };

    let mut by_month: BTreeMap<(i32, u32), Vec<&Activity>> = BTreeMap::new();
    for a in filtered {
        let key = (a.date.date().year(), a.date.date().month());
        by_month.entry(key).or_default().push(a);
    }

    by_month
        .into_iter()
        .map(|((year, month), acts)| {
            let total_dist: f64 = acts.iter().map(|a| a.distance_meters).sum();
            let total_time: f64 = acts.iter().map(|a| a.moving_time).sum();
            let total_elev: f64 = acts.iter().filter_map(|a| a.elevation_gain).sum();

            let mut unique_days = acts.iter().map(|a| a.date.date()).collect::<Vec<_>>();
            unique_days.sort();
            unique_days.dedup();

            let avg_speed = if total_time > 0.0 {
                (total_dist / total_time) * 3.6
            } else {
                0.0
            };

            MonthStats {
                year,
                month,
                total_activities: acts.len(),
                total_distance_km: total_dist / 1000.0,
                total_elevation_m: total_elev,
                total_moving_time_hours: total_time / 3600.0,
                avg_speed_kmh: avg_speed,
                ride_days: unique_days.len(),
            }
        })
        .collect()
}

#[derive(Serialize)]
pub struct Streaks {
    pub current_streak_days: usize,
    pub longest_streak_days: usize,
    pub longest_streak_start: Option<NaiveDate>,
    pub longest_streak_end: Option<NaiveDate>,
    pub current_streak_start: Option<NaiveDate>,
    pub total_active_days: usize,
    pub activity_calendar: BTreeMap<NaiveDate, usize>,
}

pub fn compute_streaks(activities: &[&Activity]) -> Streaks {
    let mut day_counts: BTreeMap<NaiveDate, usize> = BTreeMap::new();
    for a in activities {
        *day_counts.entry(a.date.date()).or_insert(0) += 1;
    }

    let total_active_days = day_counts.len();
    let days: Vec<NaiveDate> = day_counts.keys().copied().collect();

    if days.is_empty() {
        return Streaks {
            current_streak_days: 0,
            longest_streak_days: 0,
            longest_streak_start: None,
            longest_streak_end: None,
            current_streak_start: None,
            total_active_days: 0,
            activity_calendar: day_counts,
        };
    }

    let mut longest_len = 1usize;
    let mut longest_start = days[0];
    let mut longest_end = days[0];
    let mut cur_len = 1usize;
    let mut cur_start = days[0];

    for i in 1..days.len() {
        if days[i] == days[i - 1] + chrono::Duration::days(1) {
            cur_len += 1;
        } else {
            if cur_len > longest_len {
                longest_len = cur_len;
                longest_start = cur_start;
                longest_end = days[i - 1];
            }
            cur_len = 1;
            cur_start = days[i];
        }
    }
    if cur_len > longest_len {
        longest_len = cur_len;
        longest_start = cur_start;
        longest_end = *days.last().unwrap();
    }

    let last_day = *days.last().unwrap();
    let mut current_len = 1usize;
    let mut current_start = last_day;
    for i in (0..days.len() - 1).rev() {
        if days[i] == days[i + 1] - chrono::Duration::days(1) {
            current_len += 1;
            current_start = days[i];
        } else {
            break;
        }
    }

    Streaks {
        current_streak_days: current_len,
        longest_streak_days: longest_len,
        longest_streak_start: Some(longest_start),
        longest_streak_end: Some(longest_end),
        current_streak_start: Some(current_start),
        total_active_days,
        activity_calendar: day_counts,
    }
}
