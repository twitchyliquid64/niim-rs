use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use niim_core::label::{Fonts, Gray, Label, Page, render};
use niim_core::{
    Bitmap, Discovered, LabelSize, ModelInfo, Paper, PrintOptions, Printer, Progress, discover,
};

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
    /// Print a label written in simple markup: text, QR codes, barcodes, images
    #[command(long_about = LABEL_HELP)]
    Label(LabelArgs),
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

const LABEL_HELP: &str = r"Print a label written in simple markup.

Plain text prints as it is, as large as fits the label. \n or a real newline starts
a new line. Tags in square brackets change what follows:

  [left] [center] [right]   horizontal alignment from here on (default: center); used
                            mid-line it starts a new part of the line: [left]Name[right]Date
  [top] [middle] [bottom]   vertical alignment (default: middle)
  [small] [normal] [big]    text size, relative to the auto-fitted size
  [size:4mm]                fixed font size (mm, pt or px), exempt from auto-fit
  [b] ... [/b]              bold
  [col]                     start a new column to the right
  [qr:DATA]                 QR code
  [code128:DATA]            barcodes; also [ean13:DIGITS] [ean8:DIGITS] [upca:DIGITS]
  [img:FILE.png]            image, dithered; [img threshold:FILE.png] for logos

Codes and images take an optional height: [qr 10mm:DATA] [img 8mm:logo.png].
Write \[ \] \\ for literal brackets and backslashes.

Examples:
  niimctl label 'Hello'
  niimctl label -p D101 '[b]Spare keys[/b]\nGarage'
  niimctl label '[qr:https://example.com][col][left][big]Shelf 4\nBin 12'
  niimctl label --size 50x30 --preview out.png '[left]Flour[right]2026-11\n[center][code128:PANTRY-07]'

The label size comes from the loaded roll where it is known; otherwise pass --size.
With --preview and --size (or --model) no printer is needed.";

#[derive(Args)]
struct LabelArgs {
    #[command(flatten)]
    select: SelectArgs,
    /// The label markup (`-` reads it from standard input)
    #[arg(required_unless_present = "file")]
    markup: Option<String>,
    /// Read the markup from a file instead
    #[arg(short, long, conflicts_with = "markup")]
    file: Option<PathBuf>,
    /// Label size as read, WIDTHxHEIGHT in mm, e.g. `40x12` (default: detected from the roll)
    #[arg(short, long)]
    size: Option<LabelSize>,
    /// Write the label to this PNG instead of printing it
    #[arg(long, value_name = "PNG")]
    preview: Option<PathBuf>,
    /// Preview for this printer model (e.g. `B1`, `D101`) without connecting to it
    #[arg(long, requires = "preview")]
    model: Option<String>,
    /// TrueType/OpenType font to use instead of the built-in one
    #[arg(long, value_name = "FILE")]
    font: Option<PathBuf>,
    /// Font for bold text (default with --font: the same font, thickened)
    #[arg(long, value_name = "FILE", requires = "font")]
    bold_font: Option<PathBuf>,
    /// Print density (default: the model's default)
    #[arg(short, long)]
    density: Option<u8>,
    /// Number of copies
    #[arg(short = 'n', long, default_value_t = 1)]
    copies: u16,
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

/// Decode an image to 8-bit luminance, row-major. Transparent pixels count as white.
fn load_luma(path: &Path) -> AnyResult<(usize, usize, Vec<u8>)> {
    let rgba = image::open(path)?.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    let luma = rgba
        .pixels()
        .map(|p| {
            let [r, g, b, a] = p.0;
            let l = (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000;
            let a = u32::from(a);
            ((l * a + 255 * (255 - a)) / 255) as u8
        })
        .collect();
    Ok((w, h, luma))
}

fn load_png(path: &Path, threshold: u8) -> AnyResult<Bitmap> {
    let (w, h, luma) = load_luma(path)?;
    Ok(Bitmap::from_luma(w, h, &luma, threshold))
}

fn print_options(density: Option<u8>, copies: u16) -> PrintOptions {
    PrintOptions {
        density,
        copies,
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
    }
}

async fn print(args: &PrintArgs) -> AnyResult<()> {
    let bitmap = load_png(&args.image, args.threshold)?;
    let p = connect(&args.select).await?;
    let opts = print_options(args.density, args.copies);
    if args.raw {
        p.print_raw(&bitmap, &opts).await?;
    } else {
        p.print(&bitmap, &opts).await?;
    }
    eprintln!("done");
    p.disconnect().await?;
    Ok(())
}

/// Write a label to a PNG: black on white, with the area outside `safe` (which may not print)
/// in light grey.
fn save_preview(bitmap: &Bitmap, page: &Page, path: &Path) -> AnyResult<()> {
    let s = page.safe;
    let img = image::GrayImage::from_fn(bitmap.width() as u32, bitmap.height() as u32, |x, y| {
        let (x, y) = (x as usize, y as usize);
        let safe = (s.x..s.x + s.width).contains(&x) && (s.y..s.y + s.height).contains(&y);
        image::Luma([match (bitmap.get(x, y), safe) {
            (true, _) => 0,
            (false, true) => 255,
            (false, false) => 215,
        }])
    });
    img.save(path)?;
    Ok(())
}

async fn label(args: &LabelArgs) -> AnyResult<()> {
    let markup = match (&args.file, args.markup.as_deref()) {
        (Some(path), _) => std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?,
        (None, Some("-")) => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            s
        }
        (None, markup) => markup.unwrap_or_default().to_owned(),
    };
    let label = Label::parse(&markup)?;
    if label.is_empty() {
        return Err("the label is empty".into());
    }
    let read = |path: &PathBuf| {
        std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
    };
    let fonts = match &args.font {
        Some(path) => {
            let bold = args.bold_font.as_ref().map(read).transpose()?;
            Fonts::from_bytes(&read(path)?, bold.as_deref())?
        }
        None => Fonts::builtin(),
    };

    // A preview needs no printer if the size is given, or the model implies one.
    let model = match &args.model {
        Some(name) => Some(
            ModelInfo::by_name(name).ok_or_else(|| format!("unknown printer model `{name}`"))?,
        ),
        None => None,
    };
    let offline_size = args.size.or(model.and_then(ModelInfo::default_label));
    let (printer, page) = match (&args.preview, offline_size) {
        (Some(_), Some(size)) => {
            let dots_per_mm = f32::from(model.map_or(203, |m| m.dpi)) / 25.4;
            let margins = model.map(ModelInfo::margins).unwrap_or_default();
            if model.is_none() {
                eprintln!(
                    "note: previewing at 203 dpi with 1 mm margins; pass --model for a printer's \
                     exact printable area"
                );
            }
            (None, Page::new(size, dots_per_mm, margins))
        }
        (Some(_), None) if model.is_some() => {
            return Err(
                "this model's label size isn't known: pass --size, e.g. --size 40x12".into(),
            );
        }
        _ => {
            let p = connect(&args.select).await?;
            let size = match args.size {
                Some(size) => size,
                None => match p.detect_label_size().await? {
                    Some(size) => {
                        eprintln!("label: {} x {} mm", size.width_mm, size.height_mm);
                        size
                    }
                    None => {
                        p.disconnect().await?;
                        return Err(
                            "can't tell the label size from this printer or roll: pass --size, \
                             e.g. --size 40x12"
                                .into(),
                        );
                    }
                },
            };
            let page = p.page(size);
            (Some(p), page)
        }
    };

    let images = |path: &str| {
        load_luma(Path::new(path))
            .map(|(w, h, luma)| Gray::new(w, h, luma))
            .map_err(|e| e.to_string())
    };
    let rendered = match render(&label, &page, &fonts, &images) {
        Ok(r) => r,
        Err(e) => {
            if let Some(p) = &printer {
                p.disconnect().await?;
            }
            return Err(e.into());
        }
    };
    for w in &rendered.warnings {
        eprintln!("warning: {w}");
    }
    if let Some(path) = &args.preview {
        save_preview(&rendered.bitmap, &page, path)?;
        eprintln!(
            "wrote {} ({}x{} px; grey = outside the printable area)",
            path.display(),
            page.width,
            page.height
        );
    } else if let Some(p) = &printer {
        p.print(&rendered.bitmap, &print_options(args.density, args.copies))
            .await?;
        eprintln!("done");
    }
    if let Some(p) = &printer {
        p.disconnect().await?;
    }
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
        Command::Label(args) => label(&args).await,
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
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("warn,btleplug=off"),
    )
    .init();
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
