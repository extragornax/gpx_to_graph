use chrono::NaiveDateTime;
use csv::StringRecord;
use serde::Serialize;
use std::fmt;

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ActivityType {
    Ride,
    VirtualRide,
    EBikeRide,
    Walk,
    Run,
    Hike,
    Swim,
    AlpineSki,
    BackcountrySki,
    Workout,
    Yoga,
    Surfing,
    RollerSki,
    Kayaking,
    StandUpPaddling,
    InlineSkate,
    Other(String),
}

impl fmt::Display for ActivityType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ride => write!(f, "Ride"),
            Self::VirtualRide => write!(f, "Virtual Ride"),
            Self::EBikeRide => write!(f, "E-Bike Ride"),
            Self::Walk => write!(f, "Walk"),
            Self::Run => write!(f, "Run"),
            Self::Hike => write!(f, "Hike"),
            Self::Swim => write!(f, "Swim"),
            Self::AlpineSki => write!(f, "Alpine Ski"),
            Self::BackcountrySki => write!(f, "Backcountry Ski"),
            Self::Workout => write!(f, "Workout"),
            Self::Yoga => write!(f, "Yoga"),
            Self::Surfing => write!(f, "Surfing"),
            Self::RollerSki => write!(f, "Roller Ski"),
            Self::Kayaking => write!(f, "Kayaking"),
            Self::StandUpPaddling => write!(f, "Stand Up Paddling"),
            Self::InlineSkate => write!(f, "Inline Skate"),
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}

impl ActivityType {
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "Ride" => Self::Ride,
            "Virtual Ride" => Self::VirtualRide,
            "E-Bike Ride" => Self::EBikeRide,
            "Walk" => Self::Walk,
            "Run" => Self::Run,
            "Hike" => Self::Hike,
            "Swim" => Self::Swim,
            "Alpine Ski" => Self::AlpineSki,
            "Backcountry Ski" => Self::BackcountrySki,
            "Workout" => Self::Workout,
            "Yoga" => Self::Yoga,
            "Surfing" => Self::Surfing,
            "Roller Ski" => Self::RollerSki,
            "Kayaking" => Self::Kayaking,
            "Stand Up Paddling" => Self::StandUpPaddling,
            "Inline Skate" => Self::InlineSkate,
            other => Self::Other(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Activity {
    pub id: u64,
    pub date: NaiveDateTime,
    pub name: String,
    pub activity_type: ActivityType,

    pub elapsed_time: f64,
    pub moving_time: f64,

    pub distance_meters: f64,
    pub elevation_gain: Option<f64>,
    pub elevation_loss: Option<f64>,
    pub elevation_low: Option<f64>,
    pub elevation_high: Option<f64>,

    pub max_speed: Option<f64>,
    pub average_speed: f64,

    pub max_heart_rate: Option<f64>,
    pub average_heart_rate: Option<f64>,

    pub max_watts: Option<f64>,
    pub average_watts: Option<f64>,
    pub weighted_average_power: Option<f64>,

    pub max_cadence: Option<f64>,
    pub average_cadence: Option<f64>,

    pub calories: Option<f64>,
    pub relative_effort: Option<f64>,
    pub perceived_exertion: Option<f64>,
    pub training_load: Option<f64>,
    pub intensity: Option<f64>,

    pub weather_condition: Option<String>,
    pub weather_temperature: Option<f64>,
    pub apparent_temperature: Option<f64>,
    pub humidity: Option<f64>,
    pub wind_speed: Option<f64>,
    pub wind_gust: Option<f64>,
    pub wind_bearing: Option<f64>,
    pub precipitation_intensity: Option<f64>,
    pub precipitation_probability: Option<f64>,
    pub cloud_cover: Option<f64>,

    pub gear_name: Option<String>,
    pub commute: bool,
    pub athlete_weight: Option<f64>,
    pub max_grade: Option<f64>,
    pub average_grade: Option<f64>,
}

fn opt_f64(record: &StringRecord, idx: usize) -> Option<f64> {
    record.get(idx).and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            trimmed.parse().ok()
        }
    })
}

fn opt_str(record: &StringRecord, idx: usize) -> Option<String> {
    record.get(idx).and_then(|s| {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn parse_date(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s.trim(), "%b %d, %Y, %l:%M:%S %p").ok()
}

impl Activity {
    pub fn from_record(record: &StringRecord) -> Option<Self> {
        let id: u64 = record.get(0)?.trim().parse().ok()?;
        let date = parse_date(record.get(1)?)?;
        let name = record.get(2).unwrap_or("").trim().to_string();
        let activity_type = ActivityType::parse(record.get(3).unwrap_or(""));

        let elapsed_time = opt_f64(record, 15).or_else(|| opt_f64(record, 5)).unwrap_or(0.0);
        let moving_time = opt_f64(record, 16).unwrap_or(elapsed_time);
        let distance_meters = opt_f64(record, 17).or_else(|| opt_f64(record, 6)).unwrap_or(0.0);
        let average_speed = opt_f64(record, 19).unwrap_or(0.0);

        let commute_str = record.get(50).unwrap_or("").trim().to_lowercase();
        let commute = commute_str == "true" || commute_str == "1";

        Some(Self {
            id,
            date,
            name,
            activity_type,
            elapsed_time,
            moving_time,
            distance_meters,
            elevation_gain: opt_f64(record, 20),
            elevation_loss: opt_f64(record, 21),
            elevation_low: opt_f64(record, 22),
            elevation_high: opt_f64(record, 23),
            max_speed: opt_f64(record, 18),
            average_speed,
            max_heart_rate: opt_f64(record, 30),
            average_heart_rate: opt_f64(record, 31),
            max_watts: opt_f64(record, 32),
            average_watts: opt_f64(record, 33),
            weighted_average_power: opt_f64(record, 46),
            max_cadence: opt_f64(record, 28),
            average_cadence: opt_f64(record, 29),
            calories: opt_f64(record, 34),
            relative_effort: opt_f64(record, 37),
            perceived_exertion: opt_f64(record, 43),
            training_load: opt_f64(record, 88),
            intensity: opt_f64(record, 89),
            weather_condition: opt_str(record, 55),
            weather_temperature: opt_f64(record, 56),
            apparent_temperature: opt_f64(record, 57),
            humidity: opt_f64(record, 59),
            wind_speed: opt_f64(record, 61),
            wind_gust: opt_f64(record, 62),
            wind_bearing: opt_f64(record, 63),
            precipitation_intensity: opt_f64(record, 64),
            precipitation_probability: opt_f64(record, 70),
            cloud_cover: opt_f64(record, 72),
            gear_name: opt_str(record, 11),
            commute,
            athlete_weight: opt_f64(record, 13),
            max_grade: opt_f64(record, 24),
            average_grade: opt_f64(record, 25),
        })
    }
}
