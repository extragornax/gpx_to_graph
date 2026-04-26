use std::io::BufReader;

use anyhow::{bail, Context, Result};
use gpx::read;
use plotters::coord::Shift;
use plotters::prelude::*;

#[cfg(feature = "server")]
pub mod meteo;
#[cfg(feature = "server")]
pub mod ravito;
#[cfg(feature = "server")]
pub mod trace;
#[cfg(feature = "server")]
pub mod strava_stats;
#[cfg(feature = "server")]
pub mod col;

// ---------------------------------------------------------------------------
// Public data types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RawPoint {
    pub lat: f64,
    pub lon: f64,
    pub ele: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct ProfilePoint {
    pub km: f64,
    pub ele: f64,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub km: f64,
}

#[derive(Debug, Clone)]
pub struct Climb {
    pub start_km: f64,
    pub end_km: f64,
    pub gain: f64,
    pub gradient: f64,
}

/// Options that control graph generation.
pub struct GraphOptions {
    pub km_step: f64,
    pub km_label_step: f64,
    pub km_label_scale: i32,
    pub mirror: bool,
    pub checkpoint_filter: Option<String>,
    pub climb_min_gain: f64,
    pub split: Option<f64>,
}

/// The result of a full generation run.
pub struct GeneratedOutput {
    /// Each entry is a human-readable label (e.g. "0-200km") and PNG bytes.
    pub graph_images: Vec<(String, Vec<u8>)>,
    /// PNG bytes for the climb-stats table, if there were climbs.
    pub climb_stats: Option<Vec<u8>>,
    pub total_km: f64,
    pub num_checkpoints: usize,
    pub num_climbs: usize,
}

// ---------------------------------------------------------------------------
// High-level entry point
// ---------------------------------------------------------------------------

/// Run the full pipeline: parse GPX, build profile, detect climbs, render
/// graphs, and return everything as in-memory PNG images.
pub fn generate(gpx_reader: impl std::io::Read, opts: &GraphOptions) -> Result<GeneratedOutput> {
    let (track_points, waypoints) = parse_gpx(gpx_reader)?;
    let profile = build_profile(track_points);
    let checkpoints = project_checkpoints(&profile, waypoints, opts.checkpoint_filter.as_deref());
    let climbs = detect_climbs(&profile, opts.climb_min_gain);
    let total_km = profile.last().map(|p| p.km).unwrap_or(0.0);
    let y_range = compute_elevation_range(&profile);

    let mut graph_images: Vec<(String, Vec<u8>)> = Vec::new();

    if let Some(split_km) = opts.split {
        let mut start = 0.0;
        while start < total_km {
            let end = (start + split_km).min(total_km);
            let label = format!("{:.0}-{:.0}km", start, end);
            let mut png = render_graph_to_bytes(
                &profile,
                &checkpoints,
                &climbs,
                (start, end),
                y_range,
                opts.km_step,
                opts.km_label_step,
                opts.km_label_scale,
            )?;
            if opts.mirror {
                png = mirror_image_bytes(png)?;
            }
            graph_images.push((label, png));
            start += split_km;
        }
    } else {
        let label = format!("0-{:.0}km", total_km);
        let mut png = render_graph_to_bytes(
            &profile,
            &checkpoints,
            &climbs,
            (0.0, total_km),
            y_range,
            opts.km_step,
            opts.km_label_step,
            opts.km_label_scale,
        )?;
        if opts.mirror {
            png = mirror_image_bytes(png)?;
        }
        graph_images.push((label, png));
    }

    let climb_stats = if !climbs.is_empty() {
        Some(render_climb_stats_to_bytes(&climbs)?)
    } else {
        None
    };

    Ok(GeneratedOutput {
        graph_images,
        climb_stats,
        total_km,
        num_checkpoints: checkpoints.len(),
        num_climbs: climbs.len(),
    })
}

// ---------------------------------------------------------------------------
// GPX parsing
// ---------------------------------------------------------------------------

#[allow(clippy::type_complexity)]
pub fn parse_gpx(reader: impl std::io::Read) -> Result<(Vec<RawPoint>, Vec<(String, RawPoint)>)> {
    let gpx = read(BufReader::new(reader)).context("failed to parse GPX")?;

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

// ---------------------------------------------------------------------------
// Profile building & analysis
// ---------------------------------------------------------------------------

pub fn build_profile(track_points: Vec<RawPoint>) -> Vec<ProfilePoint> {
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

pub fn project_checkpoints(
    profile: &[ProfilePoint],
    waypoints: Vec<(String, RawPoint)>,
    checkpoint_filter: Option<&str>,
) -> Vec<Checkpoint> {
    let filter = checkpoint_filter.map(|s| s.to_lowercase());

    let mut out = Vec::new();
    for (name, point) in waypoints {
        if let Some(ref f) = filter
            && !name.to_lowercase().contains(f)
        {
            continue;
        }

        if let Some((_, nearest)) = profile
            .iter()
            .map(|p| (haversine_km(point.lat, point.lon, p.lat, p.lon), p))
            .min_by(|a, b| a.0.total_cmp(&b.0))
        {
            out.push(Checkpoint { km: nearest.km });
        }
    }

    out.sort_by(|a, b| a.km.total_cmp(&b.km));
    out
}

pub fn compute_elevation_range(profile: &[ProfilePoint]) -> (f64, f64) {
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
    (y_min - y_pad, y_max + y_pad)
}

fn resample_profile(profile: &[ProfilePoint], step_km: f64) -> Vec<(f64, f64)> {
    let x_max = profile.last().map(|p| p.km).unwrap_or(0.0);
    let half = step_km;
    let mut result = Vec::new();
    let mut km = 0.0;
    while km <= x_max + 1e-9 {
        let lo = (km - half).max(0.0);
        let hi = km + half;
        let mut sum = 0.0;
        let mut count = 0u32;
        for p in profile.iter() {
            if p.km < lo {
                continue;
            }
            if p.km > hi {
                break;
            }
            sum += p.ele;
            count += 1;
        }
        if count > 0 {
            result.push((km, sum / count as f64));
        }
        km += step_km;
    }
    result
}

pub fn detect_climbs(profile: &[ProfilePoint], min_gain: f64) -> Vec<Climb> {
    if profile.len() < 2 {
        return Vec::new();
    }

    let pts = resample_profile(profile, 0.2);
    if pts.len() < 2 {
        return Vec::new();
    }

    let end_drop = 15.0;
    let flat_dist = 1.0; // km without new high -> climb is over

    let mut climbs = Vec::new();
    let mut low_km = pts[0].0;
    let mut low_ele = pts[0].1;
    let mut high_km = pts[0].0;
    let mut high_ele = pts[0].1;
    let mut in_climb = false;

    let end_climb =
        |climbs: &mut Vec<Climb>, low_km: f64, low_ele: f64, high_km: f64, high_ele: f64| {
            let gain = high_ele - low_ele;
            if gain >= min_gain {
                let dist = high_km - low_km;
                let gradient = if dist > 0.001 {
                    gain / (dist * 10.0)
                } else {
                    0.0
                };
                climbs.push(Climb {
                    start_km: low_km,
                    end_km: high_km,
                    gain,
                    gradient,
                });
            }
        };

    for &(km, ele) in &pts[1..] {
        if !in_climb {
            if ele < low_ele {
                low_km = km;
                low_ele = ele;
                high_km = km;
                high_ele = ele;
            }
            if ele > high_ele {
                high_km = km;
                high_ele = ele;
            }
            if high_ele - low_ele >= min_gain {
                in_climb = true;
            }
        } else {
            if ele > high_ele {
                high_km = km;
                high_ele = ele;
            }
            let dropped = high_ele - ele >= end_drop;
            let flat = km - high_km >= flat_dist;
            if dropped || flat {
                end_climb(&mut climbs, low_km, low_ele, high_km, high_ele);
                in_climb = false;
                low_km = km;
                low_ele = ele;
                high_km = km;
                high_ele = ele;
            }
        }
    }

    if in_climb {
        let gain = high_ele - low_ele;
        let dist = high_km - low_km;
        let gradient = if dist > 0.001 {
            gain / (dist * 10.0)
        } else {
            0.0
        };
        climbs.push(Climb {
            start_km: low_km,
            end_km: high_km,
            gain,
            gradient,
        });
    }

    // Discard gentle slopes that aren't real climbs.
    climbs.retain(|c| c.gradient >= 1.0);

    climbs
}

// ---------------------------------------------------------------------------
// In-memory rendering
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn render_graph_to_bytes(
    profile: &[ProfilePoint],
    checkpoints: &[Checkpoint],
    climbs: &[Climb],
    km_range: (f64, f64),
    y_range: (f64, f64),
    km_step: f64,
    km_label_step: f64,
    km_label_scale: i32,
) -> Result<Vec<u8>> {
    let width: u32 = 2400;
    let height: u32 = 800;

    let margin_left: u32 = 80;
    let margin_top: u32 = 20;
    let margin_bottom: u32 = 400;
    let margin_right: u32 = 20;

    let plot_left = margin_left;
    let plot_top = margin_top;
    let plot_right = width - margin_right;
    let plot_bottom = height - margin_bottom;

    let mut pixel_buf = vec![0u8; (width * height * 3) as usize];
    {
        let root =
            BitMapBackend::with_buffer(&mut pixel_buf, (width, height)).into_drawing_area();
        root.fill(&WHITE)?;

        let x_min = km_range.0;
        let x_max = km_range.1.max(x_min + 1.0);
        let x_span = x_max - x_min;
        let (y_min, y_max) = y_range;

        let map_x = |km: f64| -> i32 {
            let t = (km - x_min) / x_span;
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

        // Horizontal elevation gridlines + left-side labels.
        let ele_step = nice_elevation_step(y_max - y_min);
        let ele_label_scale = 2;
        let mut ele = (y_min / ele_step).ceil() * ele_step;
        while ele <= y_max {
            let y = map_y(ele);
            if y >= plot_top as i32 && y <= plot_bottom as i32 {
                root.draw(&PathElement::new(
                    vec![(plot_left as i32, y), (plot_right as i32, y)],
                    RGBAColor(160, 160, 160, 0.3),
                ))?;
                let label = format!("{:.0}m", ele);
                draw_text_horizontal(
                    &root,
                    2,
                    y - 4 * ele_label_scale,
                    &label,
                    &RGBColor(100, 100, 100),
                    ele_label_scale,
                )?;
            }
            ele += ele_step;
        }

        // Vertical km gridlines + sideways labels.
        let km_label_y = (plot_bottom + 16) as i32;
        let mut marker = (x_min / km_step).ceil() * km_step;
        let mut label_marker = (x_min / km_label_step).ceil() * km_label_step;
        while marker <= x_max {
            let x = map_x(marker);
            root.draw(&PathElement::new(
                vec![(x, plot_top as i32), (x, plot_bottom as i32)],
                RGBAColor(160, 160, 160, 0.35),
            ))?;

            if marker + 1e-6 >= label_marker {
                let label = format!("{marker:.0}km");
                draw_text_rotated_cw(
                    &root,
                    x - (7 * km_label_scale / 2),
                    km_label_y,
                    &label,
                    &RGBColor(80, 80, 80),
                    km_label_scale,
                )?;
                label_marker += km_label_step;
            }

            marker += km_step;
        }

        // Filled area under elevation profile (only visible segment).
        let seg_profile: Vec<&ProfilePoint> = profile
            .iter()
            .filter(|p| p.km >= x_min && p.km <= x_max)
            .collect();
        let profile_pixels: Vec<(i32, i32)> = seg_profile
            .iter()
            .map(|p| (map_x(p.km), map_y(p.ele)))
            .collect();
        if profile_pixels.len() >= 2 {
            let mut fill_points = profile_pixels.clone();
            fill_points.push((map_x(x_max), plot_bottom as i32));
            fill_points.push((map_x(x_min), plot_bottom as i32));
            root.draw(&Polygon::new(fill_points, BLUE.mix(0.15)))?;
        }

        // Climb highlights (orange fill + label).
        let climb_fill_color = RGBAColor(220, 120, 20, 0.3);
        let climb_label_color = RGBColor(180, 90, 0);
        let climb_label_scale = 2;
        for climb in climbs
            .iter()
            .filter(|c| c.end_km > x_min && c.start_km < x_max)
        {
            let mut fill: Vec<(i32, i32)> = seg_profile
                .iter()
                .filter(|p| p.km >= climb.start_km && p.km <= climb.end_km)
                .map(|p| (map_x(p.km), map_y(p.ele)))
                .collect();
            if fill.len() >= 2 {
                let fill_end = climb.end_km.min(x_max);
                let fill_start = climb.start_km.max(x_min);
                fill.push((map_x(fill_end), plot_bottom as i32));
                fill.push((map_x(fill_start), plot_bottom as i32));
                root.draw(&Polygon::new(fill, climb_fill_color))?;
            }

            let peak_ele = seg_profile
                .iter()
                .filter(|p| p.km >= climb.start_km && p.km <= climb.end_km)
                .map(|p| p.ele)
                .fold(f64::NEG_INFINITY, f64::max);
            let mid_km = ((climb.start_km.max(x_min)) + (climb.end_km.min(x_max))) / 2.0;
            let label = format!("+{:.0}m {:.1}%", climb.gain, climb.gradient);
            let label_px_width = label.len() as i32 * 6 * climb_label_scale;
            let label_x = (map_x(mid_km) - label_px_width / 2).clamp(
                plot_left as i32,
                (plot_right as i32 - label_px_width).max(plot_left as i32),
            );
            let label_y = (map_y(peak_ele) - 18).max(plot_top as i32 + 2);
            draw_text_horizontal(
                &root,
                label_x,
                label_y,
                &label,
                &climb_label_color,
                climb_label_scale,
            )?;
        }

        // Checkpoint lines + labels (in a separate row below km labels).
        let checkpoint_label_scale = (km_label_scale - 1).max(2);
        let longest_km_chars = format!("{:.0}km", x_max).len() as i32;
        let km_row_height = longest_km_chars * 6 * km_label_scale;
        let checkpoint_label_y = km_label_y + km_row_height + 12;
        for cp in checkpoints
            .iter()
            .filter(|c| c.km >= x_min && c.km <= x_max)
        {
            let x = map_x(cp.km);
            root.draw(&PathElement::new(
                vec![(x, plot_top as i32), (x, plot_bottom as i32)],
                RED.mix(0.7).stroke_width(2),
            ))?;
            root.draw(&PathElement::new(
                vec![(x, plot_bottom as i32), (x, (plot_bottom + 10) as i32)],
                RED.mix(0.7).stroke_width(2),
            ))?;

            let label = format!("{:.1}km", cp.km);
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

        // End-of-segment distance label in the checkpoint row.
        let total_label = format!("{x_max:.1}km");
        let total_x = map_x(x_max);
        let total_width = 7 * checkpoint_label_scale;
        let total_min_x = margin_left as i32;
        let total_max_x = (width - margin_right) as i32 - total_width;
        let total_label_x =
            (total_x - total_width / 2).clamp(total_min_x, total_max_x.max(total_min_x));
        draw_text_rotated_cw(
            &root,
            total_label_x,
            checkpoint_label_y,
            &total_label,
            &BLACK,
            checkpoint_label_scale,
        )?;

        // Elevation profile line (on top of fill and gridlines).
        if profile_pixels.len() >= 2 {
            root.draw(&PathElement::new(
                profile_pixels,
                BLUE.mix(0.9).stroke_width(2),
            ))?;
        }

        // Plot border.
        root.draw(&Rectangle::new(
            [
                (plot_left as i32, plot_top as i32),
                (plot_right as i32, plot_bottom as i32),
            ],
            BLACK.stroke_width(1),
        ))?;

        root.present()?;
    }

    // Encode raw RGB pixel buffer to PNG.
    let img = image::RgbImage::from_raw(width, height, pixel_buf)
        .context("failed to create image from pixel buffer")?;
    let mut png_bytes = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut png_bytes),
        image::ImageOutputFormat::Png,
    )?;
    Ok(png_bytes)
}

pub fn render_climb_stats_to_bytes(climbs: &[Climb]) -> Result<Vec<u8>> {
    let scale = 3;
    let title_scale = 4;
    let char_h = 7 * scale;
    let row_h = char_h + 11;
    let pad = 20;

    let title_y = pad;
    let header_y = title_y + 7 * title_scale + 20;
    let sep_y = header_y + char_h + 4;
    let data_y = sep_y + 8;
    let summary_y = data_y + climbs.len() as i32 * row_h + 10;

    let img_w: u32 = 800;
    let img_h: u32 = (summary_y + row_h + pad) as u32;

    let mut pixel_buf = vec![0u8; (img_w * img_h * 3) as usize];
    {
        let root =
            BitMapBackend::with_buffer(&mut pixel_buf, (img_w, img_h)).into_drawing_area();
        root.fill(&WHITE)?;

        // Title.
        draw_text_horizontal(&root, pad, title_y, "CLIMB STATS", &BLACK, title_scale)?;

        // Header.
        draw_text_horizontal(
            &root,
            pad,
            header_y,
            " START   END    DIST  GAIN GRADE",
            &RGBColor(100, 100, 100),
            scale,
        )?;

        // Separator.
        root.draw(&PathElement::new(
            vec![(pad, sep_y), (img_w as i32 - pad, sep_y)],
            RGBAColor(160, 160, 160, 0.5),
        ))?;

        // Data rows.
        let mut total_gain = 0.0;
        for (i, c) in climbs.iter().enumerate() {
            let y = data_y + i as i32 * row_h;
            total_gain += c.gain;
            let line = format!(
                "{:>7} {:>7} {:>5} {:>5} {:>5}",
                format!("{:.1}km", c.start_km),
                format!("{:.1}km", c.end_km),
                format!("{:.1}", c.end_km - c.start_km),
                format!("+{:.0}m", c.gain),
                format!("{:.1}%", c.gradient),
            );
            draw_text_horizontal(&root, pad, y, &line, &BLACK, scale)?;
        }

        // Summary separator + line.
        root.draw(&PathElement::new(
            vec![
                (pad, summary_y - 4),
                (img_w as i32 - pad, summary_y - 4),
            ],
            RGBAColor(160, 160, 160, 0.5),
        ))?;
        let summary = format!("{} CLIMBS  +{:.0}m TOTAL", climbs.len(), total_gain);
        draw_text_horizontal(&root, pad, summary_y, &summary, &BLACK, scale)?;

        root.present()?;
    }

    // Encode raw RGB pixel buffer to PNG.
    let img = image::RgbImage::from_raw(img_w, img_h, pixel_buf)
        .context("failed to create image from pixel buffer")?;
    let mut png_bytes = Vec::new();
    img.write_to(
        &mut std::io::Cursor::new(&mut png_bytes),
        image::ImageOutputFormat::Png,
    )?;
    Ok(png_bytes)
}

/// Flip a PNG image horizontally, returning new PNG bytes.
pub fn mirror_image_bytes(png_bytes: Vec<u8>) -> Result<Vec<u8>> {
    let img = image::load_from_memory(&png_bytes)?;
    let flipped = image::imageops::flip_horizontal(&img);
    let mut out = Vec::new();
    flipped.write_to(
        &mut std::io::Cursor::new(&mut out),
        image::ImageOutputFormat::Png,
    )?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Bitmap text drawing helpers (private)
// ---------------------------------------------------------------------------

fn draw_text_horizontal(
    area: &DrawingArea<BitMapBackend<'_>, Shift>,
    x: i32,
    y: i32,
    text: &str,
    color: &RGBColor,
    scale: i32,
) -> Result<()> {
    let mut cursor_x = x;
    for ch in text.chars() {
        draw_char_horizontal(area, cursor_x, y, ch, color, scale)?;
        cursor_x += 6 * scale;
    }
    Ok(())
}

fn draw_char_horizontal(
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
            if (bits >> (4 - col)) & 1 == 1 {
                let px = x + col * scale;
                let py = y + row as i32 * scale;
                area.draw(&Rectangle::new(
                    [(px, py), (px + scale - 1, py + scale - 1)],
                    color.filled(),
                ))?;
            }
        }
    }
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
            let ry = col;
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
        'a' | 'A' => [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'b' | 'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        'c' | 'C' => [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110],
        'd' | 'D' => [0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100],
        'e' | 'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        'g' | 'G' => [0b01110, 0b10001, 0b10000, 0b10110, 0b10001, 0b10001, 0b01110],
        'h' | 'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        'i' | 'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        'k' | 'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        'l' | 'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        'm' | 'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        'n' | 'N' => [0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001],
        'o' | 'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        'r' | 'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        's' | 'S' => [0b01110, 0b10001, 0b10000, 0b01110, 0b00001, 0b10001, 0b01110],
        't' | 'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        '.' => [0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b00110, 0b00110],
        '+' => [0b00000, 0b00100, 0b00100, 0b11111, 0b00100, 0b00100, 0b00000],
        '-' => [0b00000, 0b00000, 0b00000, 0b11111, 0b00000, 0b00000, 0b00000],
        '%' => [0b11001, 0b11010, 0b00010, 0b00100, 0b01000, 0b01011, 0b10011],
        ':' => [0b00000, 0b01100, 0b01100, 0b00000, 0b01100, 0b01100, 0b00000],
        _ => [0b00000; 7],
    }
}

fn nice_elevation_step(range: f64) -> f64 {
    if range <= 0.0 {
        return 50.0;
    }
    let rough = range / 5.0;
    let mag = 10.0_f64.powf(rough.log10().floor());
    let r = rough / mag;
    let nice = if r <= 1.5 {
        1.0
    } else if r <= 3.5 {
        2.0
    } else if r <= 7.5 {
        5.0
    } else {
        10.0
    };
    nice * mag
}

pub fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6371.0;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    r * c
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
