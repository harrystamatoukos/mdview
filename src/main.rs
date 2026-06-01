use anyhow::{Result, bail};
use clap::Parser;
use std::path::{Path, PathBuf};

mod chart;
#[cfg(feature = "rich")]
mod filetree;
mod highlight;
mod mermaid;
mod pager;
mod parser;
mod renderer;
#[cfg(feature = "rich")]
mod rich;
mod theme;
mod view;
mod watcher;

use theme::{Theme, ThemeType};

/// A beautiful, read-only terminal markdown reader.
#[derive(Parser, Debug)]
#[command(name = "mdview")]
#[command(version, about, long_about = None)]
struct Args {
    /// The markdown file to view, or a directory to browse (defaults to the
    /// current directory). The graphical reader shows a file-tree sidebar.
    #[arg(default_value = ".")]
    path: PathBuf,

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

/// Modes that operate on a single document reject a directory argument; the
/// directory sidebar is exclusive to the graphical reader.
fn require_file(path: &Path, mode: &str) -> Result<PathBuf> {
    if path.is_dir() {
        bail!(
            "{mode} needs a markdown file, but {} is a directory.\n\
             Pass a file, or run `mdview <dir>` for the graphical browser.",
            path.display()
        );
    }
    Ok(path.to_path_buf())
}

fn main() -> Result<()> {
    let args = Args::parse();
    let theme = Theme::new(args.theme);

    #[cfg(feature = "rich")]
    if let Some(out) = args.export_png.as_ref() {
        let file = require_file(&args.path, "--export-png")?;
        let content = std::fs::read_to_string(&file)?;
        let base_dir = file.parent().map(|p| p.to_path_buf());
        rich::export_png(&content, out, base_dir)?;
        println!("Wrote {}", out.display());
        return Ok(());
    }

    if args.print {
        let file = require_file(&args.path, "--print")?;
        print!("{}", renderer::render(&std::fs::read_to_string(&file)?));
        return Ok(());
    }

    if args.watch {
        let file = require_file(&args.path, "--watch")?;
        return watcher::watch_and_display(&file, theme);
    }

    if args.tui {
        let file = require_file(&args.path, "--tui")?;
        return pager::run(&std::fs::read_to_string(&file)?, theme);
    }

    // Default: the graphical (rich) reader with a directory sidebar, falling
    // back to the classic text reader when a graphics-capable terminal isn't
    // available (or when the rich feature isn't compiled in).
    #[cfg(feature = "rich")]
    {
        // A directory browses its tree; a file opens with its siblings shown.
        let (root, focus) = if args.path.is_dir() {
            (args.path.clone(), None)
        } else {
            let root = args
                .path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            let focus = args
                .path
                .file_name()
                .map(|name| root.join(name))
                .unwrap_or_else(|| args.path.clone());
            (root, Some(focus))
        };

        match rich::run(&root, focus.as_deref()) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("rich reader unavailable ({e}); falling back to --tui.");
                // Fall back on a concrete file: the focus, else the first one
                // in the tree, else there's nothing to show.
                let file = focus.or_else(|| {
                    filetree::FileTree::build(&root, None)
                        .ok()
                        .and_then(|t| t.first_file())
                });
                match file {
                    Some(f) => pager::run(&std::fs::read_to_string(&f)?, theme),
                    None => Err(e),
                }
            }
        }
    }

    #[cfg(not(feature = "rich"))]
    {
        let file = require_file(&args.path, "the reader")?;
        pager::run(&std::fs::read_to_string(&file)?, theme)
    }
}
