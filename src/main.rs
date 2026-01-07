use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

mod document;
mod editor;
mod input;
mod parser;
mod pager;
mod position;
mod primitives;
mod renderer;
mod selection;
mod theme;
mod view;
mod watcher;

use theme::{Theme, ThemeType};

/// A beautiful terminal markdown viewer with editorial aesthetics
#[derive(Parser, Debug)]
#[command(name = "mdview")]
#[command(version, about, long_about = None)]
struct Args {
    /// The markdown file to view
    file: PathBuf,

    /// Watch for file changes and auto-refresh
    #[arg(short, long)]
    watch: bool,

    /// Print to stdout instead of pager mode
    #[arg(short, long)]
    print: bool,

    /// Color theme: paper (default), dark, light
    #[arg(short = 't', long, default_value = "paper")]
    theme: String,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let theme = Theme::new(ThemeType::from_str(&args.theme));

    // Read and parse the markdown file
    let content = std::fs::read_to_string(&args.file)?;

    if args.print {
        // Simple stdout mode
        let rendered = renderer::render(&content);
        println!("{}", rendered);
    } else if args.watch {
        // Watch mode with pager
        watcher::watch_and_display(&args.file, theme)?;
    } else {
        // Interactive pager mode (with file path for saving)
        pager::run(&content, theme, Some(args.file))?;
    }

    Ok(())
}
