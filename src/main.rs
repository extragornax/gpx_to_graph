use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Parser;
use gpx::read;
use image::io::Reader as ImageReader;
use plotters::coord::Shift;
use plotters::prelude::*;

#[derive(Parser, Debug)]
#[command(author, version, about = "Generate a bike-friendly route graph image from GPX")]
struct Args {
    /// Input GPX file
    #[arg(short, long)]
    input: PathBuf,

    /// Output image path (.png)
    #[arg(short, long, default_value = "route_graph.png")]
    output: PathBuf,

    /// Draw vertical distance markers every N kilometers
    #[arg(long, default_value_t = 10.0)]
    km_step: f64,

    /// Draw readable kilometer labels every N kilometers
    #[arg(long, default_value_t = 25.0)]
    km_label_step: f64,

    /// Scale factor for bitmap kilometer labels (2..6 is practical)
    #[arg(long, default_value_t = 5)]
    km_label_scale: i32,

    /// Mirror the whole image horizontally (useful for back-readable printing)
    #[arg(long, default_value_t = false)]
    mirror: bool,

    /// Keep only checkpoints whose name contains this text (case-insensitive)
    #[arg(long)]
    checkpoint_filter: Option<String>,
}

#[derive(Debug, Clone)]
struct RawPoint {
    lat: f64,
    lon: f64,
    ele: Option<f64>,
}

#[derive(Debug, Clone)]
struct ProfilePoint {
    km: f64,
    ele: f64,
    lat: f64,
    lon: f64,
}

#[derive(Debug, Clone)]
struct Checkpoint {
    km: f64,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if args.km_step <= 0.0 {
        bail!("--km-step must be > 0");
    }
    if args.km_label_step <= 0.0 {
        bail!("--km-label-step must be > 0");
    }
    if !(1..=8).contains(&args.km_label_scale) {
        bail!("--km-label-scale must be between 1 and 8");
    }

    let (track_points, waypoints) = parse_gpx(&args.input)?;
    let profile = build_profile(track_points);
    let checkpoints = project_checkpoints(&profile, waypoints, args.checkpoint_filter.as_deref());

    render_graph(
        &profile,
        &checkpoints,
        args.km_step,
        args.km_label_step,
        args.km_label_scale,
        &args.output,
    )?;

    if args.mirror {
        mirror_image_horizontally(&args.output)?;
    }

    println!(
        "Generated {} (distance: {:.1} km, checkpoints: {})",
        args.output.display(),
        profile.last().map(|p| p.km).unwrap_or(0.0),
        checkpoints.len()
    );

    Ok(())
}

fn parse_gpx(path: &PathBuf) -> Result<(Vec<RawPoint>, Vec<(String, RawPoint)>)> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let reader = BufReader::new(file);
    let gpx = read(reader).with_context(|| format!("failed to parse {}", path.display()))?;

    let mut track_points = Vec::new();
    for track in gpx.tracks {
        for segment in track.segments {
            for p in segment.points {
                let pt = p.point();
                track_points.push(RawPoint {
                    lat: pt.y(),
                    lon: pt.x(),
                    ele: p.elevation,
                });
            }
        }
    }

    if track_points.len() < 2 {
        bail!("GPX has no usable track points");
    }

    let mut waypoints = Vec::new();
    for w in gpx.waypoints {
        let pt = w.point();
        waypoints.push((
            w.name.unwrap_or_else(|| "checkpoint".to_string()),
            RawPoint {
                lat: pt.y(),
                lon: pt.x(),
                ele: w.elevation,
            },
        ));
    }

    Ok((track_points, waypoints))
}

fn build_profile(track_points: Vec<RawPoint>) -> Vec<ProfilePoint> {
    let mut profile = Vec::with_capacity(track_points.len());

    let mut total_km = 0.0;
    let mut last = &track_points[0];
    profile.push(ProfilePoint {
        km: 0.0,
        ele: track_points[0].ele.unwrap_or(0.0),
        lat: track_points[0].lat,
        lon: track_points[0].lon,
    });

    for p in track_points.iter().skip(1) {
        total_km += haversine_km(last.lat, last.lon, p.lat, p.lon);
        profile.push(ProfilePoint {
            km: total_km,
            ele: p.ele.unwrap_or(profile.last().map(|x| x.ele).unwrap_or(0.0)),
            lat: p.lat,
            lon: p.lon,
        });
        last = p;
    }

    profile
}

fn project_checkpoints(
    profile: &[ProfilePoint],
    waypoints: Vec<(String, RawPoint)>,
    checkpoint_filter: Option<&str>,
) -> Vec<Checkpoint> {
    let filter = checkpoint_filter.map(|s| s.to_lowercase());

    let mut out = Vec::new();
    for (name, point) in waypoints {
        if let Some(ref f) = filter {
            if !name.to_lowercase().contains(f) {
                continue;
            }
        }

        if let Some((_, nearest)) = profile
            .iter()
            .map(|p| (haversine_km(point.lat, point.lon, p.lat, p.lon), p))
            .min_by(|a, b| a.0.total_cmp(&b.0))
        {
            out.push(Checkpoint {
                km: nearest.km,
            });
        }
    }

    out.sort_by(|a, b| a.km.total_cmp(&b.km));
    out
}

fn render_graph(
    profile: &[ProfilePoint],
    checkpoints: &[Checkpoint],
    km_step: f64,
    km_label_step: f64,
    km_label_scale: i32,
    output: &PathBuf,
) -> Result<()> {
    let width = 1800;
    let height = 430;

    let margin_left = 20;
    let margin_top = 20;
    let margin_bottom = 180;
    let margin_right = 20;

    let plot_left = margin_left;
    let plot_top = margin_top;
    let plot_right = width - margin_right;
    let plot_bottom = height - margin_bottom;

    let root = BitMapBackend::new(output, (width, height)).into_drawing_area();
    root.fill(&WHITE)?;

    let x_max = profile.last().map(|p| p.km).unwrap_or(1.0).max(1.0);
    let (mut y_min, mut y_max) = profile
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(mn, mx), p| {
            (mn.min(p.ele), mx.max(p.ele))
        });

    if !y_min.is_finite() || !y_max.is_finite() {
        y_min = 0.0;
        y_max = 100.0;
    }
    if (y_max - y_min).abs() < 5.0 {
        y_max += 5.0;
        y_min -= 5.0;
    }
    let y_pad = ((y_max - y_min) * 0.1).max(10.0);
    y_min -= y_pad;
    y_max += y_pad;

    let map_x = |km: f64| -> i32 {
        let t = if x_max > 0.0 { km / x_max } else { 0.0 };
        (plot_left as f64 + t * (plot_right - plot_left) as f64).round() as i32
    };
    let map_y = |ele: f64| -> i32 {
        let t = if (y_max - y_min).abs() > f64::EPSILON {
            (ele - y_min) / (y_max - y_min)
        } else {
            0.5
        };
        (plot_bottom as f64 - t * (plot_bottom - plot_top) as f64).round() as i32
    };

    // Kilometer guide lines + sideways labels near the bottom for post-rotation readability.
    let mut marker = 0.0;
    let mut label_marker = 0.0;
    while marker <= x_max {
        let x = map_x(marker);
        root.draw(&PathElement::new(
            vec![(x, plot_top as i32), (x, plot_bottom as i32)],
            RGBAColor(80, 80, 80, 0.18),
        ))?;

        if marker + 1e-6 >= label_marker {
            let label = format!("{marker:.0}km");
            draw_text_rotated_cw(
                &root,
                x - (7 * km_label_scale / 2),
                (plot_bottom + 16) as i32,
                &label,
                &BLACK,
                km_label_scale,
            )?;
            label_marker += km_label_step;
        }

        marker += km_step;
    }

    // Checkpoint lines across full plot height + exact checkpoint kilometer labels on the border line.
    let checkpoint_label_scale = (km_label_scale - 2).max(2);
    let checkpoint_label_y = (plot_bottom + 8) as i32;
    for cp in checkpoints {
        let x = map_x(cp.km);
        root.draw(&PathElement::new(
            vec![(x, plot_top as i32), (x, plot_bottom as i32)],
            RED.mix(0.55).stroke_width(2),
        ))?;

        let label = format!("{:.2}km", cp.km);
        let label_width = 7 * checkpoint_label_scale;
        let min_x = margin_left as i32;
        let max_x = (width - margin_right) as i32 - label_width;
        let label_x = (x - label_width / 2).clamp(min_x, max_x.max(min_x));
        draw_text_rotated_cw(
            &root,
            label_x,
            checkpoint_label_y,
            &label,
            &RED,
            checkpoint_label_scale,
        )?;
    }

    // Show total route distance on the end border line.
    let total_label = format!("{x_max:.1}km");
    let total_x = map_x(x_max);
    let total_width = 7 * checkpoint_label_scale;
    let total_min_x = margin_left as i32;
    let total_max_x = (width - margin_right) as i32 - total_width;
    let total_label_x = (total_x - total_width / 2).clamp(total_min_x, total_max_x.max(total_min_x));
    draw_text_rotated_cw(
        &root,
        total_label_x,
        checkpoint_label_y,
        &total_label,
        &BLACK,
        checkpoint_label_scale,
    )?;

    // Elevation profile.
    let profile_pixels: Vec<(i32, i32)> = profile.iter().map(|p| (map_x(p.km), map_y(p.ele))).collect();
    root.draw(&PathElement::new(
        profile_pixels,
        BLUE.mix(0.95).stroke_width(3),
    ))?;

    root.present()?;
    Ok(())
}

fn draw_text_rotated_cw(
    area: &DrawingArea<BitMapBackend<'_>, Shift>,
    x: i32,
    y: i32,
    text: &str,
    color: &RGBColor,
    scale: i32,
) -> Result<()> {
    let mut cursor_y = y;
    for ch in text.chars() {
        draw_char_rotated_cw(area, x, cursor_y, ch, color, scale)?;
        cursor_y += 6 * scale;
    }
    Ok(())
}

fn draw_char_rotated_cw(
    area: &DrawingArea<BitMapBackend<'_>, Shift>,
    x: i32,
    y: i32,
    ch: char,
    color: &RGBColor,
    scale: i32,
) -> Result<()> {
    let glyph = glyph_5x7(ch);
    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..5 {
            let on = (bits >> (4 - col)) & 1 == 1;
            if !on {
                continue;
            }

            // Rotate 5x7 glyph 90deg clockwise (output size: 7x5).
            let rx = (6 - row) as i32;
            let ry = col as i32;
            let px = x + rx * scale;
            let py = y + ry * scale;

            area.draw(&Rectangle::new(
                [(px, py), (px + scale - 1, py + scale - 1)],
                color.filled(),
            ))?;
        }
    }
    Ok(())
}

fn glyph_5x7(ch: char) -> [u8; 7] {
    match ch {
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110],
        '6' => [0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110],
        'k' | 'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'm' | 'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        '.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00110, 0b00110],
        _ => [0b00000; 7],
    }
}

fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6371.0;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    r * c
}

fn mirror_image_horizontally(path: &PathBuf) -> Result<()> {
    let mut img = ImageReader::open(path)
        .with_context(|| format!("failed to open image {}", path.display()))?
        .decode()
        .with_context(|| format!("failed to decode image {}", path.display()))?;
    image::imageops::flip_horizontal_in_place(&mut img);
    img.save(path)
        .with_context(|| format!("failed to save mirrored image {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::haversine_km;

    #[test]
    fn haversine_zero_distance() {
        assert!(haversine_km(48.0, 2.0, 48.0, 2.0) < 1e-9);
    }

    #[test]
    fn haversine_known_distance_paris_london() {
        let d = haversine_km(48.8566, 2.3522, 51.5074, -0.1278);
        assert!(d > 300.0 && d < 400.0);
    }
}
