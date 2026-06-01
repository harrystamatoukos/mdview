use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

mod chart;
mod parser;
mod renderer;
#[cfg(feature = "rich")]
mod rich;
mod theme;
mod view;
mod pager;
mod watcher;

use theme::{Theme, ThemeType};

/// A beautiful, read-only terminal markdown reader.
#[derive(Parser, Debug)]
#[command(name = "mdview")]
#[command(version, about, long_about = None)]
struct Args {
    /// The markdown file to view
    file: PathBuf,

    /// Watch for file changes and auto-refresh
    #[arg(short, long)]
    watch: bool,

    /// Print to stdout instead of opening a reader
    #[arg(short, long)]
    print: bool,

    /// Use the classic text reader instead of the graphical (rich) reader
    #[arg(long)]
    tui: bool,

    /// Color theme
    #[arg(short = 't', long, value_enum, default_value = "paper")]
    theme: ThemeType,

    /// (rich) Export the rendered page to a PNG file instead of displaying.
    /// Useful for previewing the typography without a graphics terminal.
    #[cfg(feature = "rich")]
    #[arg(long, value_name = "PATH")]
    export_png: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let theme = Theme::new(args.theme);
    let content = std::fs::read_to_string(&args.file)?;

    #[cfg(feature = "rich")]
    if let Some(out) = args.export_png.as_ref() {
        let base_dir = args.file.parent().map(|p| p.to_path_buf());
        rich::export_png(&content, out, base_dir)?;
        println!("Wrote {}", out.display());
        return Ok(());
    }

    if args.print {
        print!("{}", renderer::render(&content));
        return Ok(());
    }

    if args.watch {
        return watcher::watch_and_display(&args.file, theme);
    }

    if args.tui {
        return pager::run(&content, theme);
    }

    // Default: the graphical (rich) reader, falling back to the classic text
    // reader when a graphics-capable terminal isn't available (or when the
    // rich feature isn't compiled in).
    #[cfg(feature = "rich")]
    {
        let base_dir = args.file.parent().map(|p| p.to_path_buf());
        match rich::run(&content, base_dir) {
            Ok(()) => return Ok(()),
            Err(e) => {
                eprintln!("rich reader unavailable ({e}); falling back to --tui.");
            }
        }
    }

    pager::run(&content, theme)
}
