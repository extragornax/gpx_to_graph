use super::super::models::Activity;
use chrono::{Datelike, NaiveDate};
use serde::Serialize;
use std::collections::BTreeMap;

// --- Trends ---

#[derive(Serialize)]
pub struct TrendPoint {
    pub date: NaiveDate,
    pub value: f64,
    pub rolling_avg: Option<f64>,
}

pub fn compute_trends(
    activities: &[&Activity],
    metric: &str,
    window_days: i64,
) -> Vec<TrendPoint> {
    let extract: fn(&Activity) -> Option<f64> = match metric {
        "heart_rate" => |a| a.average_heart_rate,
        "watts" => |a| a.average_watts,
        "distance" => |a| Some(a.distance_meters / 1000.0),
        _ => |a| Some(a.average_speed * 3.6),
    };

    let mut by_date: BTreeMap<NaiveDate, Vec<f64>> = BTreeMap::new();
    for a in activities {
        if let Some(v) = extract(a) {
            by_date.entry(a.date.date()).or_default().push(v);
        }
    }

    let points: Vec<(NaiveDate, f64)> = by_date
        .into_iter()
        .map(|(date, vals)| {
            let avg = vals.iter().sum::<f64>() / vals.len() as f64;
            (date, avg)
        })
        .collect();

    points
        .iter()
        .enumerate()
        .map(|(i, &(date, value))| {
            let cutoff = date - chrono::Duration::days(window_days);
            let window_vals: Vec<f64> = points[..=i]
                .iter()
                .filter(|(d, _)| *d > cutoff)
                .map(|(_, v)| *v)
                .collect();

            let rolling_avg = if window_vals.len() >= 2 {
                Some(window_vals.iter().sum::<f64>() / window_vals.len() as f64)
            } else {
                None
            };

            TrendPoint {
                date,
                value,
                rolling_avg,
            }
        })
        .collect()
}

// --- Personal Bests ---

#[derive(Serialize)]
pub struct PersonalBest {
    pub activity_id: u64,
    pub date: NaiveDate,
    pub activity_name: String,
    pub value: f64,
}

#[derive(Serialize)]
pub struct PersonalBests {
    pub longest_distance_km: Option<PersonalBest>,
    pub most_elevation_m: Option<PersonalBest>,
    pub fastest_avg_speed_kmh: Option<PersonalBest>,
    pub highest_avg_watts: Option<PersonalBest>,
    pub longest_moving_time_hours: Option<PersonalBest>,
    pub most_calories: Option<PersonalBest>,
    pub highest_heart_rate_bpm: Option<PersonalBest>,
}

fn best_by<F: Fn(&Activity) -> Option<f64>>(activities: &[&Activity], f: F) -> Option<PersonalBest> {
    activities
        .iter()
        .filter_map(|a| {
            f(a).map(|v| PersonalBest {
                activity_id: a.id,
                date: a.date.date(),
                activity_name: a.name.clone(),
                value: v,
            })
        })
        .max_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal))
}

pub fn compute_personal_bests(activities: &[&Activity]) -> PersonalBests {
    PersonalBests {
        longest_distance_km: best_by(activities, |a| Some(a.distance_meters / 1000.0)),
        most_elevation_m: best_by(activities, |a| a.elevation_gain),
        fastest_avg_speed_kmh: best_by(activities, |a| {
            let kmh = a.average_speed * 3.6;
            if kmh > 0.0 { Some(kmh) } else { None }
        }),
        highest_avg_watts: best_by(activities, |a| a.average_watts),
        longest_moving_time_hours: best_by(activities, |a| Some(a.moving_time / 3600.0)),
        most_calories: best_by(activities, |a| a.calories),
        highest_heart_rate_bpm: best_by(activities, |a| a.max_heart_rate),
    }
}

// --- Fitness Curve (Banister CTL/ATL/TSB) ---

#[derive(Serialize)]
pub struct FitnessPoint {
    pub date: NaiveDate,
    pub fitness_ctl: f64,
    pub fatigue_atl: f64,
    pub form_tsb: f64,
    pub daily_load: f64,
}

pub fn compute_fitness_curve(
    activities: &[&Activity],
    ctl_days: f64,
    atl_days: f64,
) -> Vec<FitnessPoint> {
    let mut daily_load: BTreeMap<NaiveDate, f64> = BTreeMap::new();
    for a in activities {
        let load = a.training_load.or(a.relative_effort).unwrap_or(0.0);
        *daily_load.entry(a.date.date()).or_insert(0.0) += load;
    }

    if daily_load.is_empty() {
        return vec![];
    }

    let first = *daily_load.keys().next().unwrap();
    let last = *daily_load.keys().last().unwrap();

    let ctl_decay = (-1.0 / ctl_days).exp();
    let atl_decay = (-1.0 / atl_days).exp();

    let mut ctl = 0.0f64;
    let mut atl = 0.0f64;
    let mut result = Vec::new();

    let mut day = first;
    while day <= last {
        let load = daily_load.get(&day).copied().unwrap_or(0.0);

        ctl = ctl * ctl_decay + load * (1.0 - ctl_decay);
        atl = atl * atl_decay + load * (1.0 - atl_decay);

        if daily_load.contains_key(&day) || day == last {
            result.push(FitnessPoint {
                date: day,
                fitness_ctl: (ctl * 10.0).round() / 10.0,
                fatigue_atl: (atl * 10.0).round() / 10.0,
                form_tsb: ((ctl - atl) * 10.0).round() / 10.0,
                daily_load: load,
            });
        }

        day += chrono::Duration::days(1);
    }

    result
}

// --- Power Curve (monthly progression) ---

#[derive(Serialize)]
pub struct PowerMonth {
    pub month: String,
    pub avg_watts: f64,
    pub max_avg_watts: f64,
    pub weighted_avg_power: Option<f64>,
    pub count: usize,
}

pub fn compute_power_curve(activities: &[&Activity]) -> Vec<PowerMonth> {
    let mut by_month: BTreeMap<(i32, u32), Vec<&Activity>> = BTreeMap::new();
    for a in activities {
        if a.average_watts.is_some() {
            let key = (a.date.date().year(), a.date.date().month());
            by_month.entry(key).or_default().push(a);
        }
    }

    by_month
        .into_iter()
        .map(|((year, month), acts)| {
            let watts: Vec<f64> = acts.iter().filter_map(|a| a.average_watts).collect();
            let wap: Vec<f64> = acts.iter().filter_map(|a| a.weighted_average_power).collect();

            PowerMonth {
                month: format!("{year}-{month:02}"),
                avg_watts: watts.iter().sum::<f64>() / watts.len() as f64,
                max_avg_watts: watts.iter().fold(0.0f64, |a, &b| a.max(b)),
                weighted_avg_power: if wap.is_empty() {
                    None
                } else {
                    Some(wap.iter().sum::<f64>() / wap.len() as f64)
                },
                count: acts.len(),
            }
        })
        .collect()
}

// --- HR Zones ---

#[derive(Serialize)]
pub struct HrZone {
    pub zone: u8,
    pub label: String,
    pub range_bpm: (f64, f64),
    pub activity_count: usize,
}

#[derive(Serialize)]
pub struct HrBucket {
    pub bucket_bpm: u32,
    pub count: usize,
}

#[derive(Serialize)]
pub struct HrZones {
    pub max_hr_used: f64,
    pub zones: Vec<HrZone>,
    pub avg_hr_distribution: Vec<HrBucket>,
}

pub fn compute_hr_zones(activities: &[&Activity], user_max_hr: Option<f64>) -> HrZones {
    let observed_max = activities
        .iter()
        .filter_map(|a| a.max_heart_rate)
        .fold(0.0f64, f64::max);

    let max_hr = user_max_hr.unwrap_or(observed_max).max(1.0);

    let zone_defs: [(u8, &str, f64, f64); 5] = [
        (1, "Recovery", 0.0, 0.6),
        (2, "Endurance", 0.6, 0.7),
        (3, "Tempo", 0.7, 0.8),
        (4, "Threshold", 0.8, 0.9),
        (5, "VO2max", 0.9, 1.0),
    ];

    let with_hr: Vec<f64> = activities
        .iter()
        .filter_map(|a| a.average_heart_rate)
        .collect();

    let zones: Vec<HrZone> = zone_defs
        .iter()
        .map(|&(zone, label, low_pct, high_pct)| {
            let low = max_hr * low_pct;
            let high = max_hr * high_pct;
            let count = with_hr.iter().filter(|&&hr| hr >= low && hr < high).count();
            HrZone {
                zone,
                label: label.to_string(),
                range_bpm: (low.round(), high.round()),
                activity_count: count,
            }
        })
        .collect();

    let mut buckets: BTreeMap<u32, usize> = BTreeMap::new();
    for &hr in &with_hr {
        let bucket = (hr / 5.0).floor() as u32 * 5;
        *buckets.entry(bucket).or_insert(0) += 1;
    }

    let avg_hr_distribution: Vec<HrBucket> = buckets
        .into_iter()
        .map(|(bucket_bpm, count)| HrBucket { bucket_bpm, count })
        .collect();

    HrZones {
        max_hr_used: max_hr,
        zones,
        avg_hr_distribution,
    }
}
