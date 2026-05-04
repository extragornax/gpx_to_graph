use std::f64::consts::PI;
use std::io::BufReader;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use rand::Rng;
use serde::{Deserialize, Serialize};

// ── Public types ──

#[derive(Debug, Clone, Serialize)]
pub struct RouteResult {
    pub gpx: String,
    pub stats: RouteStats,
    pub waypoints: Vec<[f64; 2]>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteStats {
    pub distance_km: f64,
    pub dplus_m: f64,
    pub dminus_m: f64,
    pub estimated_duration_h: f64,
    pub dominant_direction: String,
    pub greenway_pct: f64,
    pub elevations: Vec<[f64; 2]>,
}

#[derive(Debug, Deserialize)]
pub struct GenerateRequest {
    pub start: [f64; 2],
    pub distance_km: f64,
    pub dplus_max: Option<f64>,
    pub profile: Option<String>,
    #[serde(rename = "loop")]
    pub is_loop: Option<bool>,
    pub waypoints: Option<Vec<[f64; 2]>>,
    pub avoid_session: Option<String>,
}

// ── Waypoint generation ──

pub fn generate_loop_waypoints(
    start_lat: f64,
    start_lon: f64,
    distance_km: f64,
    direction_deg: f64,
    forced: &[[f64; 2]],
) -> Vec<(f64, f64)> {
    let mut rng = rand::rng();
    let n = ((distance_km / 30.0) as usize).clamp(2, 8);
    let radius_km = distance_km / (2.0 * PI);
    let radius_deg = radius_km / 111.0;
    let dir_rad = direction_deg.to_radians();

    let mut wps: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            let angle = dir_rad + 2.0 * PI * (i as f64) / (n as f64);
            let perturbation: f64 = rng.random_range(0.7..1.3);
            let r = radius_deg * perturbation;
            let lat = start_lat + r * angle.cos();
            let lon = start_lon + r * angle.sin() / start_lat.to_radians().cos();
            (lat, lon)
        })
        .collect();

    insert_forced(&mut wps, forced);
    wps
}

pub fn generate_oneway_waypoints(
    start_lat: f64,
    start_lon: f64,
    distance_km: f64,
    direction_deg: f64,
    forced: &[[f64; 2]],
) -> Vec<(f64, f64)> {
    let mut rng = rand::rng();
    let n = ((distance_km / 30.0) as usize).clamp(2, 8);
    let total_deg = distance_km / 111.0;
    let dir_rad = direction_deg.to_radians();

    let mut wps: Vec<(f64, f64)> = (1..=n)
        .map(|i| {
            let frac = i as f64 / (n as f64 + 1.0);
            let lateral: f64 = rng.random_range(-0.2..0.2);
            let along = total_deg * frac;
            let lat = start_lat + along * dir_rad.cos() + lateral * total_deg * dir_rad.sin();
            let lon = start_lon
                + (along * dir_rad.sin() + lateral * total_deg * dir_rad.cos())
                    / start_lat.to_radians().cos();
            (lat, lon)
        })
        .collect();

    insert_forced(&mut wps, forced);
    wps
}

fn insert_forced(wps: &mut Vec<(f64, f64)>, forced: &[[f64; 2]]) {
    for fw in forced {
        let best = wps
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                let da = (a.0 - fw[0]).powi(2) + (a.1 - fw[1]).powi(2);
                let db = (b.0 - fw[0]).powi(2) + (b.1 - fw[1]).powi(2);
                da.partial_cmp(&db).unwrap()
            })
            .map(|(i, _)| i)
            .unwrap_or(0);
        wps.insert(best + 1, (fw[0], fw[1]));
    }
}

fn scale_waypoints(wps: &[(f64, f64)], center: (f64, f64), factor: f64) -> Vec<(f64, f64)> {
    wps.iter()
        .map(|(lat, lon)| {
            (
                center.0 + (lat - center.0) * factor,
                center.1 + (lon - center.1) * factor,
            )
        })
        .collect()
}

// ── BRouter client ──

pub async fn route_via_brouter(
    client: &reqwest::Client,
    brouter_url: &str,
    needs_rate_limit: bool,
    waypoints: &[(f64, f64)],
    profile: &str,
) -> Result<Vec<BRouterSegment>> {
    let mut segments = Vec::new();

    for pair in waypoints.windows(2) {
        let lonlats = format!(
            "{:.6},{:.6}|{:.6},{:.6}",
            pair[0].1, pair[0].0, pair[1].1, pair[1].0
        );
        let url = format!(
            "{}?lonlats={}&profile={}&alternativeidx=0&format=gpx",
            brouter_url, lonlats, profile
        );

        if needs_rate_limit {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        let resp = client
            .get(&url)
            .header("User-Agent", "roulette-velo/1.0 (extragornax.fr)")
            .send()
            .await
            .context("BRouter request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("BRouter returned {}: {}", status, body);
        }

        let gpx_bytes = resp.bytes().await.context("reading BRouter response")?;
        let parsed = gpx::read(BufReader::new(gpx_bytes.as_ref()))
            .context("parsing BRouter GPX response")?;

        let points: Vec<TrackPoint> = parsed
            .tracks
            .iter()
            .flat_map(|t| &t.segments)
            .flat_map(|s| &s.points)
            .map(|p| TrackPoint {
                lat: p.point().y(),
                lon: p.point().x(),
                ele: p.elevation.unwrap_or(0.0),
            })
            .collect();

        segments.push(BRouterSegment { points });
    }

    Ok(segments)
}

#[derive(Debug, Clone)]
pub struct BRouterSegment {
    pub points: Vec<TrackPoint>,
}

#[derive(Debug, Clone)]
pub struct TrackPoint {
    pub lat: f64,
    pub lon: f64,
    pub ele: f64,
}

// ── GPX assembly & stats ──

pub fn assemble_gpx(segments: &[BRouterSegment]) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<gpx version="1.1" creator="roulette-velo" xmlns="http://www.topografix.com/GPX/1/1">
<trk><name>Roulette Vélo</name><trkseg>
"#,
    );

    for (i, seg) in segments.iter().enumerate() {
        let start = if i > 0 { 1 } else { 0 };
        for pt in seg.points.iter().skip(start) {
            xml.push_str(&format!(
                "<trkpt lat=\"{:.6}\" lon=\"{:.6}\"><ele>{:.1}</ele></trkpt>\n",
                pt.lat, pt.lon, pt.ele
            ));
        }
    }

    xml.push_str("</trkseg></trk>\n</gpx>");
    xml
}

pub fn compute_stats(segments: &[BRouterSegment], start: [f64; 2]) -> RouteStats {
    let all_pts: Vec<&TrackPoint> = segments
        .iter()
        .enumerate()
        .flat_map(|(i, seg)| {
            let skip = if i > 0 { 1 } else { 0 };
            seg.points.iter().skip(skip)
        })
        .collect();

    let mut distance = 0.0_f64;
    let mut dplus = 0.0_f64;
    let mut dminus = 0.0_f64;
    let mut elevations = Vec::new();
    let mut cum_dist = 0.0_f64;

    for i in 0..all_pts.len() {
        if i > 0 {
            let seg_m = haversine_m(
                all_pts[i - 1].lat,
                all_pts[i - 1].lon,
                all_pts[i].lat,
                all_pts[i].lon,
            );
            distance += seg_m;
            cum_dist = distance;

            let diff = all_pts[i].ele - all_pts[i - 1].ele;
            if diff > 0.0 {
                dplus += diff;
            } else {
                dminus += diff.abs();
            }
        }

        if i % 5 == 0 || i == all_pts.len() - 1 {
            elevations.push([cum_dist / 1000.0, all_pts[i].ele]);
        }
    }

    let distance_km = distance / 1000.0;

    let avg_grade = if distance_km > 0.0 {
        dplus / distance * 100.0
    } else {
        0.0
    };
    let speed = (20.0 - 2.0 * avg_grade).max(8.0);
    let estimated_duration_h = distance_km / speed;

    let last_pt = all_pts.last();
    let dominant_direction = match last_pt {
        Some(pt) => bearing_to_cardinal(bearing(start[0], start[1], pt.lat, pt.lon)),
        None => "N".to_string(),
    };

    RouteStats {
        distance_km,
        dplus_m: dplus,
        dminus_m: dminus,
        estimated_duration_h,
        dominant_direction,
        greenway_pct: 0.0,
        elevations,
    }
}

fn total_distance_m(segments: &[BRouterSegment]) -> f64 {
    let mut dist = 0.0;
    for (i, seg) in segments.iter().enumerate() {
        let start = if i > 0 { 1 } else { 0 };
        let pts = &seg.points;
        for j in (start + 1)..pts.len() {
            dist += haversine_m(pts[j - 1].lat, pts[j - 1].lon, pts[j].lat, pts[j].lon);
        }
    }
    dist
}

fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6_371_000.0;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    r * 2.0 * a.sqrt().atan2((1.0 - a).sqrt())
}

fn bearing(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let (lat1, lon1, lat2, lon2) = (
        lat1.to_radians(),
        lon1.to_radians(),
        lat2.to_radians(),
        lon2.to_radians(),
    );
    let dlon = lon2 - lon1;
    let y = dlon.sin() * lat2.cos();
    let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * dlon.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

fn bearing_to_cardinal(deg: f64) -> String {
    let dirs = ["N", "NE", "E", "SE", "S", "SO", "O", "NO"];
    let idx = ((deg + 22.5) / 45.0) as usize % 8;
    dirs[idx].to_string()
}

// ── Avoid logic ──

pub fn overlap_pct(
    segments: &[BRouterSegment],
    avoid_points: &[(f64, f64)],
    buffer_m: f64,
) -> f64 {
    if avoid_points.is_empty() {
        return 0.0;
    }

    let mut total_m = 0.0;
    let mut overlap_m = 0.0;

    for (i, seg) in segments.iter().enumerate() {
        let start = if i > 0 { 1 } else { 0 };
        for j in (start + 1)..seg.points.len() {
            let pt = &seg.points[j];
            let prev = &seg.points[j - 1];
            let seg_m = haversine_m(prev.lat, prev.lon, pt.lat, pt.lon);
            total_m += seg_m;

            let near = avoid_points.iter().any(|ap| {
                haversine_m(pt.lat, pt.lon, ap.0, ap.1) < buffer_m
            });
            if near {
                overlap_m += seg_m;
            }
        }
    }

    if total_m > 0.0 {
        overlap_m / total_m * 100.0
    } else {
        0.0
    }
}

// ── Main generation orchestrator ──

pub async fn generate_route(
    client: &reqwest::Client,
    brouter_url: &str,
    needs_rate_limit: bool,
    req: &GenerateRequest,
    avoid_points: &[(f64, f64)],
) -> Result<RouteResult> {
    let directions: Vec<f64> = {
        let mut rng = rand::rng();
        (0..3u32)
            .map(|attempt| {
                let base: f64 = rng.random_range(0.0..360.0);
                base + 90.0 * attempt as f64
            })
            .collect()
    };
    let profile = req.profile.as_deref().unwrap_or("trekking");
    let is_loop = req.is_loop.unwrap_or(true);
    let forced = req.waypoints.as_deref().unwrap_or(&[]);
    let start = (req.start[0], req.start[1]);
    let tolerance = 0.10;

    let mut best: Option<RouteResult> = None;
    let mut best_dist_err = f64::MAX;

    for attempt in 0..3u32 {
        let direction = directions[attempt as usize];

        let mut wps = if is_loop {
            generate_loop_waypoints(start.0, start.1, req.distance_km, direction, forced)
        } else {
            generate_oneway_waypoints(start.0, start.1, req.distance_km, direction, forced)
        };

        let mut route_wps: Vec<(f64, f64)> = vec![start];
        route_wps.extend(&wps);
        if is_loop {
            route_wps.push(start);
        }

        let segments = match route_via_brouter(client, brouter_url, needs_rate_limit, &route_wps, profile).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("BRouter attempt {} failed: {}", attempt + 1, e);
                continue;
            }
        };

        let total_m = total_distance_m(&segments);
        let total_km = total_m / 1000.0;
        let stats = compute_stats(&segments, req.start);

        let dist_err = ((total_km - req.distance_km) / req.distance_km).abs();

        let mut warnings = Vec::new();

        if dist_err > tolerance {
            let scale_factor = req.distance_km / total_km;
            wps = scale_waypoints(&wps, start, scale_factor.sqrt());

            let mut route_wps2: Vec<(f64, f64)> = vec![start];
            route_wps2.extend(&wps);
            if is_loop {
                route_wps2.push(start);
            }

            match route_via_brouter(client, brouter_url, needs_rate_limit, &route_wps2, profile).await {
                Ok(s2) => {
                    let km2 = total_distance_m(&s2) / 1000.0;
                    let stats2 = compute_stats(&s2, req.start);
                    let err2 = ((km2 - req.distance_km) / req.distance_km).abs();

                    if err2 < dist_err {
                        let mut w2 = Vec::new();
                        if err2 > tolerance {
                            w2.push(format!(
                                "Distance {:.0} km (cible {:.0} km, écart {:.0}%)",
                                km2,
                                req.distance_km,
                                err2 * 100.0
                            ));
                        }
                        if let Some(max_dp) = req.dplus_max {
                            if stats2.dplus_m > max_dp {
                                w2.push(format!(
                                    "D+ {:.0} m au-dessus du max demandé ({:.0} m)",
                                    stats2.dplus_m, max_dp
                                ));
                            }
                        }
                        let gpx = assemble_gpx(&s2);
                        let wp_out: Vec<[f64; 2]> = wps.iter().map(|w| [w.0, w.1]).collect();
                        let result = RouteResult { gpx, stats: stats2, waypoints: wp_out, warnings: w2 };
                        let e2 = err2;
                        if e2 < best_dist_err {
                            best_dist_err = e2;
                            best = Some(result);
                        }
                        if e2 <= tolerance {
                            return Ok(best.unwrap());
                        }
                        continue;
                    }
                }
                Err(e) => {
                    tracing::warn!("BRouter scale retry failed: {}", e);
                }
            }
        }

        if let Some(max_dp) = req.dplus_max {
            if stats.dplus_m > max_dp {
                warnings.push(format!(
                    "D+ {:.0} m au-dessus du max demandé ({:.0} m)",
                    stats.dplus_m, max_dp
                ));
            }
        }

        if !avoid_points.is_empty() {
            let pct = overlap_pct(&segments, avoid_points, 200.0);
            if pct > 30.0 {
                warnings.push(format!(
                    "Chevauchement de {:.0}% avec vos routes existantes",
                    pct
                ));
                if attempt < 2 {
                    continue;
                }
            }
        }

        if dist_err > tolerance {
            warnings.push(format!(
                "Distance {:.0} km (cible {:.0} km, écart {:.0}%)",
                total_km,
                req.distance_km,
                dist_err * 100.0
            ));
        }

        let gpx = assemble_gpx(&segments);
        let wp_out: Vec<[f64; 2]> = wps.iter().map(|w| [w.0, w.1]).collect();
        let result = RouteResult { gpx, stats, waypoints: wp_out, warnings };

        if dist_err < best_dist_err {
            best_dist_err = dist_err;
            best = Some(result);
        }

        if dist_err <= tolerance {
            break;
        }
    }

    best.ok_or_else(|| anyhow::anyhow!("Impossible de générer un parcours après 3 tentatives"))
}

// ── Daily route generation ──

pub const DAILY_CITIES: &[(&str, f64, f64)] = &[
    ("paris", 48.8566, 2.3522),
    ("lyon", 45.7640, 4.8357),
    ("marseille", 43.2965, 5.3698),
    ("bordeaux", 44.8378, -0.5792),
    ("toulouse", 43.6047, 1.4442),
    ("nantes", 47.2184, -1.5536),
    ("strasbourg", 48.5734, 7.7521),
    ("lille", 50.6292, 3.0573),
    ("montpellier", 43.6108, 3.8767),
    ("rennes", 48.1173, -1.6778),
];

pub fn daily_seed(date: &str) -> u64 {
    let clean: String = date.chars().filter(|c| c.is_ascii_digit()).collect();
    clean.parse().unwrap_or(20260101)
}

pub fn daily_direction(seed: u64, city_idx: usize) -> f64 {
    let mixed = seed.wrapping_mul(2654435761).wrapping_add(city_idx as u64 * 1000003);
    (mixed % 360) as f64
}
