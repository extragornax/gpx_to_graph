use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::meteo::gpx_parse::bearing_deg;
use crate::ravito::gpx_parse::haversine_km;
use crate::ravito::hours::{Openness, status_at};
use crate::ravito::overpass::Poi;

#[derive(Debug, Serialize)]
pub struct NearbyPoi {
    pub kind: String,
    pub lat: f64,
    pub lon: f64,
    pub name: Option<String>,
    pub opening_hours: Option<String>,
    pub distance_m: f64,
    /// Initial bearing from the rider to the POI, 0..360, north-up.
    pub bearing_deg: f64,
    /// "open", "closed" or "unknown" right now.
    pub status_now: String,
}

/// Keep the POIs within `radius_m` of (lat, lon) that match `kinds`, drop the
/// ones known to be closed when `open_now`, then take the `limit` nearest.
///
/// Filtering before truncating is the whole point: cut to 10 first and the open
/// filter leaves you three rows.
pub fn rank(
    pois: Vec<Poi>,
    lat: f64,
    lon: f64,
    radius_m: f64,
    kinds: Option<&HashSet<String>>,
    open_now: bool,
    limit: usize,
    now: &DateTime<Utc>,
) -> Vec<NearbyPoi> {
    let mut out = Vec::new();
    for p in pois {
        let distance_m = haversine_km(lat, lon, p.lat, p.lon) * 1000.0;
        if distance_m > radius_m {
            continue;
        }
        let kind = p.kind.as_str().to_string();
        if let Some(f) = kinds
            && !f.contains(&kind)
        {
            continue;
        }
        let status = match &p.opening_hours {
            Some(h) => match status_at(h, now) {
                Openness::Open => "open",
                Openness::Closed => "closed",
                Openness::Unknown => "unknown",
            },
            None => "unknown",
        };
        // Unknown hours survive the filter — half of OSM has none tagged, and a
        // maybe-open bakery still beats no bakery.
        if open_now && status == "closed" {
            continue;
        }
        out.push(NearbyPoi {
            kind,
            lat: p.lat,
            lon: p.lon,
            name: p.name,
            opening_hours: p.opening_hours,
            distance_m,
            bearing_deg: bearing_deg(lat, lon, p.lat, p.lon),
            status_now: status.to_string(),
        });
    }
    out.sort_by(|a, b| {
        a.distance_m
            .partial_cmp(&b.distance_m)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ravito::overpass::PoiKind;

    const LAT: f64 = 48.8566;
    const LON: f64 = 2.3522;

    fn poi(kind: PoiKind, lat: f64, lon: f64, hours: Option<&str>) -> Poi {
        Poi {
            osm_id: 1,
            kind,
            lat,
            lon,
            name: None,
            opening_hours: hours.map(String::from),
        }
    }

    /// 0.001° of latitude is ~111 m, handy for placing POIs at known distances.
    fn north(metres: f64) -> f64 {
        LAT + metres / 111_000.0
    }

    fn now() -> DateTime<Utc> {
        // Wednesday 2026-08-12, 10:00 UTC — inside a "Mo-Sa 07:00-19:00" window.
        DateTime::parse_from_rfc3339("2026-08-12T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn sorts_by_distance_and_drops_beyond_radius() {
        let pois = vec![
            poi(PoiKind::Bakery, north(800.0), LON, None),
            poi(PoiKind::Fountain, north(200.0), LON, None),
            poi(PoiKind::Supermarket, north(3000.0), LON, None),
        ];
        let got = rank(pois, LAT, LON, 1000.0, None, false, 10, &now());
        let kinds: Vec<&str> = got.iter().map(|p| p.kind.as_str()).collect();
        assert_eq!(kinds, vec!["fountain", "bakery"]);
        assert!((got[0].distance_m - 200.0).abs() < 5.0);
    }

    #[test]
    fn filters_by_kind() {
        let pois = vec![
            poi(PoiKind::Bakery, north(100.0), LON, None),
            poi(PoiKind::Cemetery, north(200.0), LON, None),
        ];
        let kinds: HashSet<String> = ["cemetery".to_string()].into_iter().collect();
        let got = rank(pois, LAT, LON, 1000.0, Some(&kinds), false, 10, &now());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].kind, "cemetery");
    }

    #[test]
    fn open_filter_applies_before_the_limit() {
        // Three closed shops sit nearer than the open one. Asking for the single
        // nearest open shop must return the open one, not an empty list.
        let pois = vec![
            poi(PoiKind::Bakery, north(100.0), LON, Some("Mo-Sa 20:00-22:00")),
            poi(PoiKind::Bakery, north(200.0), LON, Some("Mo-Sa 20:00-22:00")),
            poi(PoiKind::Bakery, north(300.0), LON, Some("Mo-Sa 20:00-22:00")),
            poi(PoiKind::Bakery, north(400.0), LON, Some("Mo-Sa 07:00-19:00")),
        ];
        let got = rank(pois, LAT, LON, 1000.0, None, true, 1, &now());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].status_now, "open");
        assert!((got[0].distance_m - 400.0).abs() < 5.0);
    }

    #[test]
    fn unknown_hours_survive_the_open_filter() {
        let pois = vec![poi(PoiKind::Fountain, north(100.0), LON, None)];
        let got = rank(pois, LAT, LON, 1000.0, None, true, 10, &now());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].status_now, "unknown");
    }

    #[test]
    fn bearing_points_north_for_a_poi_due_north() {
        let pois = vec![poi(PoiKind::Bakery, north(500.0), LON, None)];
        let got = rank(pois, LAT, LON, 1000.0, None, false, 10, &now());
        assert!(got[0].bearing_deg < 1.0 || got[0].bearing_deg > 359.0);
    }
}
