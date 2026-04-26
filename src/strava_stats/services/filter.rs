use super::super::models::{Activity, ActivityType};
use chrono::NaiveDate;
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct ActivityFilter {
    pub activity_type: Option<String>,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub gear: Option<String>,
}

pub fn filter_activities<'a>(
    activities: &'a [Activity],
    filter: &ActivityFilter,
) -> Vec<&'a Activity> {
    activities
        .iter()
        .filter(|a| {
            if let Some(ref t) = filter.activity_type
                && a.activity_type != ActivityType::parse(t)
            {
                return false;
            }
            if let Some(from) = filter.from
                && a.date.date() < from
            {
                return false;
            }
            if let Some(to) = filter.to
                && a.date.date() > to
            {
                return false;
            }
            if let Some(ref g) = filter.gear {
                match &a.gear_name {
                    Some(name) if name == g => {}
                    _ => return false,
                }
            }
            true
        })
        .collect()
}
