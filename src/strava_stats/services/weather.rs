use super::super::models::Activity;
use chrono::NaiveDate;
use serde::Serialize;
use std::collections::BTreeMap;

// --- Correlation ---

#[derive(Serialize)]
pub struct CorrelationPoint {
    pub activity_id: u64,
    pub date: NaiveDate,
    pub weather_value: f64,
    pub performance_value: f64,
}

#[derive(Serialize)]
pub struct Correlation {
    pub weather_metric: String,
    pub performance_metric: String,
    pub data_points: Vec<CorrelationPoint>,
    pub correlation_coefficient: Option<f64>,
    pub regression: Option<Regression>,
}

#[derive(Serialize)]
pub struct Regression {
    pub slope: f64,
    pub intercept: f64,
}

fn extract_weather(a: &Activity, metric: &str) -> Option<f64> {
    match metric {
        "temperature" => a.weather_temperature,
        "wind_speed" => a.wind_speed,
        "humidity" => a.humidity,
        "precipitation" => a.precipitation_intensity,
        "cloud_cover" => a.cloud_cover,
        "apparent_temperature" => a.apparent_temperature,
        _ => a.weather_temperature,
    }
}

fn extract_performance(a: &Activity, metric: &str) -> Option<f64> {
    match metric {
        "average_speed" | "speed" => Some(a.average_speed * 3.6),
        "average_watts" | "watts" => a.average_watts,
        "average_heart_rate" | "heart_rate" => a.average_heart_rate,
        "calories" => a.calories,
        _ => Some(a.average_speed * 3.6),
    }
}

fn pearson(xs: &[f64], ys: &[f64]) -> Option<f64> {
    let n = xs.len() as f64;
    if n < 3.0 {
        return None;
    }
    let sum_x: f64 = xs.iter().sum();
    let sum_y: f64 = ys.iter().sum();
    let sum_xy: f64 = xs.iter().zip(ys).map(|(x, y)| x * y).sum();
    let sum_x2: f64 = xs.iter().map(|x| x * x).sum();
    let sum_y2: f64 = ys.iter().map(|y| y * y).sum();

    let denom = ((n * sum_x2 - sum_x * sum_x) * (n * sum_y2 - sum_y * sum_y)).sqrt();
    if denom == 0.0 {
        return None;
    }
    Some((n * sum_xy - sum_x * sum_y) / denom)
}

fn linear_regression(xs: &[f64], ys: &[f64]) -> Option<Regression> {
    let n = xs.len() as f64;
    if n < 2.0 {
        return None;
    }
    let sum_x: f64 = xs.iter().sum();
    let sum_y: f64 = ys.iter().sum();
    let sum_xy: f64 = xs.iter().zip(ys).map(|(x, y)| x * y).sum();
    let sum_x2: f64 = xs.iter().map(|x| x * x).sum();

    let denom = n * sum_x2 - sum_x * sum_x;
    if denom == 0.0 {
        return None;
    }
    let slope = (n * sum_xy - sum_x * sum_y) / denom;
    let intercept = (sum_y - slope * sum_x) / n;
    Some(Regression { slope, intercept })
}

pub fn compute_correlation(
    activities: &[&Activity],
    weather_metric: &str,
    performance_metric: &str,
) -> Correlation {
    let mut points = Vec::new();
    let mut xs = Vec::new();
    let mut ys = Vec::new();

    for a in activities {
        if let (Some(w), Some(p)) = (
            extract_weather(a, weather_metric),
            extract_performance(a, performance_metric),
        ) {
            points.push(CorrelationPoint {
                activity_id: a.id,
                date: a.date.date(),
                weather_value: w,
                performance_value: p,
            });
            xs.push(w);
            ys.push(p);
        }
    }

    Correlation {
        weather_metric: weather_metric.to_string(),
        performance_metric: performance_metric.to_string(),
        correlation_coefficient: pearson(&xs, &ys),
        regression: linear_regression(&xs, &ys),
        data_points: points,
    }
}

// --- Summary by weather buckets ---

#[derive(Serialize)]
pub struct WeatherBucket {
    pub bucket: String,
    pub avg_speed_kmh: f64,
    pub avg_watts: Option<f64>,
    pub count: usize,
}

#[derive(Serialize)]
pub struct ConditionBucket {
    pub condition: String,
    pub avg_speed_kmh: f64,
    pub avg_watts: Option<f64>,
    pub count: usize,
}

#[derive(Serialize)]
pub struct WeatherSummary {
    pub by_temperature: Vec<WeatherBucket>,
    pub by_wind_speed: Vec<WeatherBucket>,
    pub by_condition: Vec<ConditionBucket>,
}

fn bucket_label(val: f64, step: f64, unit: &str) -> String {
    let low = (val / step).floor() * step;
    let high = low + step;
    format!("{low:.0}-{high:.0}{unit}")
}

fn aggregate_bucket(acts: &[&Activity]) -> (f64, Option<f64>) {
    let speeds: Vec<f64> = acts.iter().map(|a| a.average_speed * 3.6).collect();
    let avg_speed = speeds.iter().sum::<f64>() / speeds.len() as f64;
    let watts: Vec<f64> = acts.iter().filter_map(|a| a.average_watts).collect();
    let avg_watts = if watts.is_empty() {
        None
    } else {
        Some(watts.iter().sum::<f64>() / watts.len() as f64)
    };
    (avg_speed, avg_watts)
}

pub fn compute_summary(activities: &[&Activity]) -> WeatherSummary {
    // By temperature (5°C buckets)
    let mut by_temp: BTreeMap<String, Vec<&Activity>> = BTreeMap::new();
    for a in activities {
        if let Some(t) = a.weather_temperature {
            by_temp
                .entry(bucket_label(t, 5.0, "°C"))
                .or_default()
                .push(a);
        }
    }
    let by_temperature: Vec<WeatherBucket> = by_temp
        .into_iter()
        .map(|(bucket, acts)| {
            let (avg_speed, avg_watts) = aggregate_bucket(&acts);
            WeatherBucket {
                bucket,
                avg_speed_kmh: avg_speed,
                avg_watts,
                count: acts.len(),
            }
        })
        .collect();

    // By wind speed (3 m/s buckets)
    let mut by_wind: BTreeMap<String, Vec<&Activity>> = BTreeMap::new();
    for a in activities {
        if let Some(w) = a.wind_speed {
            by_wind
                .entry(bucket_label(w, 3.0, " m/s"))
                .or_default()
                .push(a);
        }
    }
    let by_wind_speed: Vec<WeatherBucket> = by_wind
        .into_iter()
        .map(|(bucket, acts)| {
            let (avg_speed, avg_watts) = aggregate_bucket(&acts);
            WeatherBucket {
                bucket,
                avg_speed_kmh: avg_speed,
                avg_watts,
                count: acts.len(),
            }
        })
        .collect();

    // By condition
    let mut by_cond: BTreeMap<String, Vec<&Activity>> = BTreeMap::new();
    for a in activities {
        if let Some(ref c) = a.weather_condition {
            by_cond.entry(c.clone()).or_default().push(a);
        }
    }
    let by_condition: Vec<ConditionBucket> = by_cond
        .into_iter()
        .map(|(condition, acts)| {
            let (avg_speed, avg_watts) = aggregate_bucket(&acts);
            ConditionBucket {
                condition,
                avg_speed_kmh: avg_speed,
                avg_watts,
                count: acts.len(),
            }
        })
        .collect();

    WeatherSummary {
        by_temperature,
        by_wind_speed,
        by_condition,
    }
}

// --- Wind Rose ---

#[derive(Serialize)]
pub struct WindSector {
    pub bearing_start: f64,
    pub bearing_end: f64,
    pub label: String,
    pub avg_speed_kmh: f64,
    pub avg_wind_speed: f64,
    pub count: usize,
}

const DIRECTIONS: [&str; 16] = [
    "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
    "NW", "NNW",
];

pub fn compute_wind_rose(activities: &[&Activity]) -> Vec<WindSector> {
    let sector_size = 360.0 / 16.0;
    let mut sectors: Vec<(Vec<f64>, Vec<f64>)> = vec![(vec![], vec![]); 16];

    for a in activities {
        if let (Some(bearing), Some(wind)) = (a.wind_bearing, a.wind_speed) {
            let idx = (((bearing + sector_size / 2.0) % 360.0) / sector_size).floor() as usize;
            let idx = idx.min(15);
            sectors[idx].0.push(a.average_speed * 3.6);
            sectors[idx].1.push(wind);
        }
    }

    sectors
        .into_iter()
        .enumerate()
        .map(|(i, (speeds, winds))| {
            let start = i as f64 * sector_size;
            let end = start + sector_size;
            let count = speeds.len();
            let avg_speed = if count > 0 {
                speeds.iter().sum::<f64>() / count as f64
            } else {
                0.0
            };
            let avg_wind = if count > 0 {
                winds.iter().sum::<f64>() / count as f64
            } else {
                0.0
            };
            WindSector {
                bearing_start: start,
                bearing_end: end,
                label: DIRECTIONS[i].to_string(),
                avg_speed_kmh: avg_speed,
                avg_wind_speed: avg_wind,
                count,
            }
        })
        .collect()
}
