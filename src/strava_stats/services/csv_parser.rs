use super::super::error::AppError;
use super::super::models::Activity;

pub fn parse_activities(data: &[u8]) -> Result<Vec<Activity>, AppError> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(data);

    let mut activities = Vec::new();
    let mut skipped = 0u32;

    for result in rdr.records() {
        let record = match result {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("Skipping malformed CSV row: {e}");
                skipped += 1;
                continue;
            }
        };

        match Activity::from_record(&record) {
            Some(activity) => activities.push(activity),
            None => {
                skipped += 1;
            }
        }
    }

    if activities.is_empty() {
        return Err(AppError::CsvParse("No valid activities found".into()));
    }

    tracing::info!(
        "Parsed {} activities ({} rows skipped)",
        activities.len(),
        skipped
    );

    activities.sort_by_key(|a| std::cmp::Reverse(a.date));
    Ok(activities)
}
