use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use niim_core::{Bitmap, Discovered, Paper, PrintOptions, Printer, Progress, discover};

#[derive(Parser)]
#[command(name = "niimctl", version)]
#[command(about = "Print labels on NIIMBOT label makers")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List printers found over Bluetooth LE and USB
    List(ScanArgs),
    /// Show printer identity and live status
    Status(SelectArgs),
    /// Print a PNG label image (as the label is read; black pixels print)
    Print(PrintArgs),
    /// Write a PNG test pattern sized for a label (orientation and placement check)
    TestLabel(TestLabelArgs),
    /// Generate shell completion script
    #[command(
        long_about = "Generate a shell tab-completion script for niimctl.\nSupported shells include bash, zsh, elvish and fish.\n\n   source <(niimctl completions bash)"
    )]
    Completions(CompletionsArgs),
}

#[derive(Args)]
struct ScanArgs {
    /// Seconds to scan for Bluetooth printers
    #[arg(long, default_value_t = 4.0)]
    scan: f32,
}

#[derive(Args)]
struct SelectArgs {
    #[command(flatten)]
    scan: ScanArgs,
    /// Use the first printer whose name contains this (case-insensitive), e.g. `D101` or a serial
    #[arg(short, long, env = "NIIMCTL_PRINTER")]
    printer: Option<String>,
}

#[derive(Args)]
struct PrintArgs {
    #[command(flatten)]
    select: SelectArgs,
    /// PNG image, in the label's reading orientation
    image: PathBuf,
    /// Print density (default: the model's default)
    #[arg(short, long)]
    density: Option<u8>,
    /// Number of copies
    #[arg(short = 'n', long, default_value_t = 1)]
    copies: u16,
    /// Luminance below which a pixel is black (0-255)
    #[arg(long, default_value_t = 128)]
    threshold: u8,
    /// Send the image as-is in printhead space (columns across the head, rows along the feed)
    #[arg(long)]
    raw: bool,
}

#[derive(Args)]
struct TestLabelArgs {
    /// Label width in mm, as read
    width_mm: f32,
    /// Label height in mm, as read
    height_mm: f32,
    /// Output PNG
    #[arg(short, long, default_value = "test-label.png")]
    out: PathBuf,
    /// Printer resolution
    #[arg(long, default_value_t = 203)]
    dpi: u16,
}

#[derive(Args)]
struct CompletionsArgs {
    /// The shell to generate completions for
    shell: Shell,
}

type AnyResult<T> = Result<T, Box<dyn std::error::Error>>;

async fn find(select: &SelectArgs) -> AnyResult<Discovered> {
    let found = discover(Duration::from_secs_f32(select.scan.scan)).await?;
    let needle = select.printer.as_deref().map(str::to_lowercase);
    found
        .into_iter()
        .find(|d| {
            needle
                .as_ref()
                .is_none_or(|n| d.name().to_lowercase().contains(n))
        })
        .ok_or_else(|| "no matching printer found".into())
}

async fn connect(select: &SelectArgs) -> AnyResult<Printer> {
    let dev = find(select).await?;
    eprintln!("connecting to {} ({})…", dev.name(), dev.kind());
    Ok(dev.connect().await?)
}

fn mac(m: [u8; 6]) -> String {
    m.map(|b| format!("{b:02X}")).join(":")
}

async fn status(select: &SelectArgs) -> AnyResult<()> {
    let p = connect(select).await?;
    let i = p.info();
    println!("name:        {}", i.name);
    println!("model:       {} (id {})", i.model_name(), i.model_id);
    println!("serial:      {}", i.serial.as_deref().unwrap_or("-"));
    println!("mac:         {}", i.mac.map_or("-".into(), mac));
    println!(
        "firmware:    {}",
        i.firmware.map_or("-".into(), |v| v.to_string())
    );
    println!(
        "hardware:    {}",
        i.hardware.map_or("-".into(), |v| v.to_string())
    );
    println!("protocol:    {}", i.protocol_version);
    println!(
        "printhead:   {} dots @ {} dpi ({:.1} mm)",
        i.head_dots,
        i.dpi,
        f32::from(i.head_dots) / i.dots_per_mm()
    );
    println!(
        "density:     {}..={} (default {})",
        i.density_range.0, i.density_range.1, i.density_default
    );
    let s = p.status().await?;
    let yn = |b: Option<bool>, y: &str, n: &str| {
        b.map_or("unknown".to_owned(), |b| if b { y } else { n }.to_owned())
    };
    println!("lid:         {}", yn(s.lid_closed, "closed", "open"));
    println!(
        "paper:       {}",
        match s.paper {
            Paper::Present => "loaded",
            Paper::Absent => "none",
            Paper::Unknown => "unknown",
        }
    );
    println!(
        "battery:     {}",
        s.battery_percent.map_or("-".into(), |b| format!("{b}%"))
    );
    println!(
        "error:       {}",
        s.error.map_or("none".into(), |e| e.to_string())
    );
    if let Some(tag) = s.consumable.or(p.consumable().await.ok().flatten()) {
        println!(
            "roll:        {} ({} of {} left, serial {})",
            tag.barcode,
            tag.remaining(),
            tag.total,
            tag.serial
        );
    }
    p.disconnect().await?;
    Ok(())
}

fn load_png(path: &PathBuf, threshold: u8) -> AnyResult<Bitmap> {
    let img = image::open(path)?;
    // Transparent pixels count as white.
    let rgba = img.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    let luma: Vec<u8> = rgba
        .pixels()
        .map(|p| {
            let [r, g, b, a] = p.0;
            let l = (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000;
            let a = u32::from(a);
            ((l * a + 255 * (255 - a)) / 255) as u8
        })
        .collect();
    Ok(Bitmap::from_luma(w, h, &luma, threshold))
}

async fn print(args: &PrintArgs) -> AnyResult<()> {
    let bitmap = load_png(&args.image, args.threshold)?;
    let p = connect(&args.select).await?;
    let opts = PrintOptions {
        density: args.density,
        copies: args.copies,
        on_progress: Some(Arc::new(|pr| match pr {
            Progress::Sending { sent, total } if sent == total => {
                eprintln!("sent {total} row packets")
            }
            Progress::Printing {
                page,
                total,
                print_percent,
                feed_percent,
            } => {
                eprintln!("page {page}/{total}: print {print_percent}%, feed {feed_percent}%")
            }
            _ => {}
        })),
        ..Default::default()
    };
    if args.raw {
        p.print_raw(&bitmap, &opts).await?;
    } else {
        p.print(&bitmap, &opts).await?;
    }
    eprintln!("done");
    p.disconnect().await?;
    Ok(())
}

/// Orientation/placement pattern: a 1-dot border, a 1 mm inset border, a large `F` in the
/// top-left quadrant (readable only when not mirrored or rotated) and an arrow pointing right.
fn test_label(args: &TestLabelArgs) -> AnyResult<()> {
    let dpmm = f32::from(args.dpi) / 25.4;
    let (w, h) = (
        (args.width_mm * dpmm).round() as usize,
        (args.height_mm * dpmm).round() as usize,
    );
    let mm = dpmm.round() as usize;
    let mut b = Bitmap::new(w, h);
    // Outer 1-dot border and 1 mm inset border.
    for (inset, t) in [(0, 1), (mm, 1)] {
        b.fill_rect(inset, inset, w - 2 * inset, t, true);
        b.fill_rect(inset, h - inset - t, w - 2 * inset, t, true);
        b.fill_rect(inset, inset, t, h - 2 * inset, true);
        b.fill_rect(w - inset - t, inset, t, h - 2 * inset, true);
    }
    // F: height = 60% of label height, left half.
    let fh = h * 6 / 10;
    let s = (fh / 6).max(2);
    let (fx, fy) = (2 * mm + s, (h - fh) / 2);
    b.fill_rect(fx, fy, s, fh, true); // stem
    b.fill_rect(fx, fy, fh * 2 / 3, s, true); // top bar
    b.fill_rect(fx, fy + fh / 2 - s / 2, fh / 2, s, true); // middle bar
    // Arrow pointing right, right half.
    let ay = h / 2;
    let (ax0, ax1) = (w / 2, w - 3 * mm);
    b.fill_rect(ax0, ay - s / 2, ax1 - ax0, s, true);
    for i in 0..(fh / 3) {
        b.fill_rect(ax1 - i, ay - i, 1, 2 * i + 1, true);
    }
    let img = image::GrayImage::from_fn(w as u32, h as u32, |x, y| {
        image::Luma([if b.get(x as usize, y as usize) {
            0
        } else {
            255
        }])
    });
    img.save(&args.out)?;
    eprintln!("wrote {} ({w}x{h} px)", args.out.display());
    Ok(())
}

async fn run(cli: Cli) -> AnyResult<()> {
    match cli.command {
        Command::List(args) => {
            let found = discover(Duration::from_secs_f32(args.scan)).await?;
            if found.is_empty() {
                eprintln!("no printers found");
            }
            for d in found {
                println!("{:<4} {}", d.kind(), d.name());
            }
            Ok(())
        }
        Command::Status(args) => status(&args).await,
        Command::Print(args) => print(&args).await,
        Command::TestLabel(args) => test_label(&args),
        Command::Completions(args) => {
            clap_complete::generate(
                args.shell,
                &mut Cli::command(),
                "niimctl",
                &mut std::io::stdout(),
            );
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,btleplug=off")).init();
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
