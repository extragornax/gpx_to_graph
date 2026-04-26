use super::super::models::Activity;
use chrono::{Datelike, NaiveDate};
use serde::Serialize;
use std::collections::BTreeMap;

// --- Weekly Training Load ---

#[derive(Serialize)]
pub struct WeekStats {
    pub week_start: NaiveDate,
    pub total_distance_km: f64,
    pub total_moving_time_hours: f64,
    pub total_elevation_m: f64,
    pub total_relative_effort: f64,
    pub total_calories: f64,
    pub activity_count: usize,
    pub monotony: Option<f64>,
    pub strain: Option<f64>,
}

fn week_start(date: NaiveDate) -> NaiveDate {
    let days_from_monday = date.weekday().num_days_from_monday();
    date - chrono::Duration::days(days_from_monday as i64)
}

pub fn compute_weekly(activities: &[&Activity]) -> Vec<WeekStats> {
    let mut by_week: BTreeMap<NaiveDate, Vec<&Activity>> = BTreeMap::new();
    for a in activities {
        let ws = week_start(a.date.date());
        by_week.entry(ws).or_default().push(a);
    }

    by_week
        .into_iter()
        .map(|(week_start_date, acts)| {
            let total_dist: f64 = acts.iter().map(|a| a.distance_meters).sum();
            let total_time: f64 = acts.iter().map(|a| a.moving_time).sum();
            let total_elev: f64 = acts.iter().filter_map(|a| a.elevation_gain).sum();
            let total_effort: f64 = acts
                .iter()
                .filter_map(|a| a.training_load.or(a.relative_effort))
                .sum();
            let total_cal: f64 = acts.iter().filter_map(|a| a.calories).sum();

            // Monotony: mean daily load / std dev daily load
            let mut daily_loads = [0.0f64; 7];
            for a in &acts {
                let day_idx = a.date.date().weekday().num_days_from_monday() as usize;
                daily_loads[day_idx] += a.training_load.or(a.relative_effort).unwrap_or(0.0);
            }
            let mean = daily_loads.iter().sum::<f64>() / 7.0;
            let variance = daily_loads.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / 7.0;
            let std_dev = variance.sqrt();

            let (monotony, strain) = if std_dev > 0.0 {
                let m = mean / std_dev;
                (Some(m), Some(total_effort * m))
            } else {
                (None, None)
            };

            WeekStats {
                week_start: week_start_date,
                total_distance_km: total_dist / 1000.0,
                total_moving_time_hours: total_time / 3600.0,
                total_elevation_m: total_elev,
                total_relative_effort: total_effort,
                total_calories: total_cal,
                activity_count: acts.len(),
                monotony,
                strain,
            }
        })
        .collect()
}

// --- Fitness / Fatigue (reuses the Banister model) ---

#[derive(Serialize)]
pub struct FitnessFatigue {
    pub current: FitnessCurrent,
    pub history: Vec<FitnessHistoryPoint>,
}

#[derive(Serialize)]
pub struct FitnessCurrent {
    pub fitness_ctl: f64,
    pub fatigue_atl: f64,
    pub form_tsb: f64,
}

#[derive(Serialize)]
pub struct FitnessHistoryPoint {
    pub date: NaiveDate,
    pub ctl: f64,
    pub atl: f64,
    pub tsb: f64,
    pub daily_load: f64,
}

pub fn compute_fitness_fatigue(
    activities: &[&Activity],
    ctl_days: f64,
    atl_days: f64,
) -> FitnessFatigue {
    let mut daily_load: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    for a in activities {
        let load = a.training_load.or(a.relative_effort).unwrap_or(0.0);
        *daily_load.entry(a.date.date()).or_insert(0.0) += load;
    }

    if daily_load.is_empty() {
        return FitnessFatigue {
            current: FitnessCurrent {
                fitness_ctl: 0.0,
                fatigue_atl: 0.0,
                form_tsb: 0.0,
            },
            history: vec![],
        };
    }

    let first = *daily_load.keys().next().unwrap();
    let last = *daily_load.keys().last().unwrap();

    let ctl_decay = (-1.0 / ctl_days).exp();
    let atl_decay = (-1.0 / atl_days).exp();

    let mut ctl = 0.0f64;
    let mut atl = 0.0f64;
    let mut history = Vec::new();

    let mut day = first;
    while day <= last {
        let load = daily_load.get(&day).copied().unwrap_or(0.0);
        ctl = ctl * ctl_decay + load * (1.0 - ctl_decay);
        atl = atl * atl_decay + load * (1.0 - atl_decay);

        if daily_load.contains_key(&day) || day == last {
            history.push(FitnessHistoryPoint {
                date: day,
                ctl: (ctl * 10.0).round() / 10.0,
                atl: (atl * 10.0).round() / 10.0,
                tsb: ((ctl - atl) * 10.0).round() / 10.0,
                daily_load: load,
            });
        }

        day += chrono::Duration::days(1);
    }

    let current = FitnessCurrent {
        fitness_ctl: (ctl * 10.0).round() / 10.0,
        fatigue_atl: (atl * 10.0).round() / 10.0,
        form_tsb: ((ctl - atl) * 10.0).round() / 10.0,
    };

    FitnessFatigue { current, history }
}

// --- Volume Trends ---

#[derive(Serialize)]
pub struct VolumePeriod {
    pub period_start: NaiveDate,
    pub distance_km: f64,
    pub moving_time_hours: f64,
    pub elevation_m: f64,
    pub activity_count: usize,
    pub avg_intensity: Option<f64>,
}

pub fn compute_volume(activities: &[&Activity], period: &str) -> Vec<VolumePeriod> {
    let key_fn: fn(NaiveDate) -> NaiveDate = match period {
        "monthly" => |d| NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap(),
        _ => week_start,
    };

    let mut by_period: BTreeMap<NaiveDate, Vec<&Activity>> = BTreeMap::new();
    for a in activities {
        let key = key_fn(a.date.date());
        by_period.entry(key).or_default().push(a);
    }

    by_period
        .into_iter()
        .map(|(period_start, acts)| {
            let total_dist: f64 = acts.iter().map(|a| a.distance_meters).sum();
            let total_time: f64 = acts.iter().map(|a| a.moving_time).sum();
            let total_elev: f64 = acts.iter().filter_map(|a| a.elevation_gain).sum();

            let intensities: Vec<f64> = acts.iter().filter_map(|a| a.intensity).collect();
            let avg_intensity = if intensities.is_empty() {
                None
            } else {
                Some(intensities.iter().sum::<f64>() / intensities.len() as f64)
            };

            VolumePeriod {
                period_start,
                distance_km: total_dist / 1000.0,
                moving_time_hours: total_time / 3600.0,
                elevation_m: total_elev,
                activity_count: acts.len(),
                avg_intensity,
            }
        })
        .collect()
}
