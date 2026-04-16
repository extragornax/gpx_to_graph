use std::fs;
use std::io::BufReader;
use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::Parser;
use gpx_to_graph::{generate, GraphOptions};

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

    /// Minimum elevation gain (meters) to highlight a climb
    #[arg(long, default_value_t = 30.0)]
    climb_min_gain: f64,

    /// Split output into separate files of N kilometers each
    #[arg(long)]
    split: Option<f64>,
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
    if let Some(split) = args.split
        && split <= 0.0
    {
        bail!("--split must be > 0");
    }

    let file = fs::File::open(&args.input)?;
    let reader = BufReader::new(file);

    let opts = GraphOptions {
        km_step: args.km_step,
        km_label_step: args.km_label_step,
        km_label_scale: args.km_label_scale,
        mirror: args.mirror,
        checkpoint_filter: args.checkpoint_filter,
        climb_min_gain: args.climb_min_gain,
        split: args.split,
    };

    let result = generate(reader, &opts)?;

    let stem = args.output.file_stem().unwrap_or_default().to_string_lossy();
    let ext = args.output.extension().unwrap_or_default().to_string_lossy();
    let parent = args.output.parent().unwrap_or(std::path::Path::new("."));

    if result.graph_images.len() == 1 {
        fs::write(&args.output, &result.graph_images[0].1)?;
        println!("Generated {}", args.output.display());
    } else {
        for (label, png_bytes) in &result.graph_images {
            let path = parent.join(format!("{stem}_{label}.{ext}"));
            fs::write(&path, png_bytes)?;
            println!("Generated {}", path.display());
        }
        println!(
            "{} files (distance: {:.1} km, checkpoints: {}, climbs: {})",
            result.graph_images.len(),
            result.total_km,
            result.num_checkpoints,
            result.num_climbs
        );
    }

    if let Some(ref stats_bytes) = result.climb_stats {
        let stats_path = parent.join(format!("{stem}_climbs.{ext}"));
        fs::write(&stats_path, stats_bytes)?;
        println!("Generated {}", stats_path.display());
    }

    if result.graph_images.len() == 1 {
        println!(
            "distance: {:.1} km, checkpoints: {}, climbs: {}",
            result.total_km, result.num_checkpoints, result.num_climbs
        );
    }

    Ok(())
}
