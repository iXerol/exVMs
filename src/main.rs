use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use exvms::db;
use exvms::export::{self, ExportReport, SelectionMode};
use exvms::storage;
use exvms::template::FilenameTemplate;
use exvms::time;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Parser)]
#[command(
    name = "exvms",
    version,
    about = "Export recordings from Voice Memos.app"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// View the Voice Memos storage path and recording metadata.
    View,
    /// Export recordings to a directory.
    Export {
        /// Destination directory.
        #[arg(short, long)]
        output: PathBuf,
        /// Stable recording ID from `exvms view`. Repeat to export multiple recordings.
        #[arg(long = "id")]
        ids: Vec<i64>,
        /// Export every local-exportable recording, skipping recordings that need download.
        #[arg(short = 'a', long)]
        all: bool,
        /// Parallel export job count. Defaults to min(4, available parallelism).
        #[arg(short = 'p', long = "parallel")]
        jobs: Option<usize>,
        /// Filename template for exported recordings.
        #[arg(short = 'f', long = "filename-format")]
        filename_format: Option<String>,
    },
}

fn main() {
    if let Err(err) = run() {
        if matches!(&err, exvms::ExvmsError::Io(io) if io.kind() == io::ErrorKind::BrokenPipe) {
            return;
        }
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> exvms::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::View => view(),
        Command::Export {
            output,
            ids,
            all,
            jobs,
            filename_format,
        } => export_cmd(output, ids, all, jobs, filename_format),
    }
}

fn view() -> exvms::Result<()> {
    let store = storage::discover()?;
    let rows = db::load_recordings(&store.db_path)?;
    let stdout = io::stdout();
    let mut out = stdout.lock();

    writeln!(out, "storage: {}", store.recordings_dir.display())?;
    writeln!(out, "database: {}", store.db_path.display())?;
    writeln!(out)?;
    write_table_row(
        &mut out,
        &[
            ("id", 6),
            ("recorded_at", 20),
            ("duration", 10),
            ("availability", 23),
            ("store", 18),
            ("path", 12),
            ("folder", 20),
            ("db_path", 28),
        ],
        "title",
    )?;

    for row in rows {
        let availability = match export::classify_availability(&store.recordings_dir, &row)? {
            export::Availability::InRecordingsDirectory => "in-recordings-directory".to_string(),
            export::Availability::NeedsDownload(reason) => format!("needs-download ({reason})"),
            export::Availability::Corrupted(reason) => format!("corrupted ({reason})"),
            export::Availability::OutsideRecordingsDir(path) => {
                format!("outside-recordings-dir ({path})")
            }
        };
        write_table_row(
            &mut out,
            &[
                (&row.id.to_string(), 6),
                (&time::format_apple_local(row.recorded_at), 20),
                (&time::format_duration(row.duration), 10),
                (&availability, 23),
                (row.store_kind.as_str(), 18),
                (row.path_kind.as_str(), 12),
                (row.folder_name.as_deref().unwrap_or(""), 20),
                (&row.db_path, 28),
            ],
            &row.title,
        )?;
    }

    Ok(())
}

fn write_table_row(mut out: impl Write, columns: &[(&str, usize)], tail: &str) -> io::Result<()> {
    for (value, width) in columns {
        write!(out, "{} ", pad_display_width(value, *width))?;
    }
    writeln!(out, "{tail}")
}

fn pad_display_width(value: &str, width: usize) -> String {
    let display_width = UnicodeWidthStr::width_cjk(value);
    if display_width >= width {
        value.to_string()
    } else {
        let mut out = String::with_capacity(value.len() + (width - display_width));
        out.push_str(value);
        out.push_str(&" ".repeat(width - display_width));
        out
    }
}

fn export_cmd(
    output: PathBuf,
    ids: Vec<i64>,
    all: bool,
    jobs: Option<usize>,
    filename_format: Option<String>,
) -> exvms::Result<()> {
    if all && !ids.is_empty() {
        return Err(exvms::ExvmsError::InvalidArguments(
            "--all cannot be combined with --id".to_string(),
        ));
    }

    if !all && ids.is_empty() && !confirm_export_all()? {
        println!("aborted");
        return Ok(());
    }

    let store = storage::discover()?;
    let rows = db::load_recordings(&store.db_path)?;
    let template = match filename_format {
        Some(value) => FilenameTemplate::parse(&value)?,
        None => FilenameTemplate::default_template(),
    };
    let mode = if ids.is_empty() {
        SelectionMode::All
    } else {
        SelectionMode::ExplicitIds
    };
    let selected = if ids.is_empty() {
        None
    } else {
        Some(ids.as_slice())
    };
    let (planned, skipped) = export::plan_exports_with_template(
        &store.recordings_dir,
        &output,
        &rows,
        selected,
        mode,
        &template,
    )?;
    let report =
        export::execute_exports(planned, skipped, jobs.unwrap_or_else(export::default_jobs));
    print_report(&report)?;

    if report.failures.is_empty() {
        Ok(())
    } else {
        std::process::exit(2);
    }
}

fn confirm_export_all() -> exvms::Result<bool> {
    print!(
        "No --id was provided. Export all local recordings and skip recordings that need download? Type yes to continue: "
    );
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim() == "yes")
}

fn print_report(report: &ExportReport) -> exvms::Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();

    writeln!(out, "export summary:")?;
    writeln!(out, "  succeeded: {}", report.successes.len())?;
    for item in &report.successes {
        writeln!(out, "    id {} -> {}", item.id, item.target.display())?;
    }

    writeln!(out, "  skipped needs-download: {}", report.skipped.len())?;
    for item in &report.skipped {
        writeln!(out, "    id {}: {}", item.id, item.reason)?;
    }

    writeln!(out, "  failed: {}", report.failures.len())?;
    for item in &report.failures {
        writeln!(out, "    id {}: {}", item.id, item.error)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_display_width_counts_cjk_as_double_width() {
        let padded = pad_display_width("録音", 6);
        assert_eq!(UnicodeWidthStr::width_cjk(padded.as_str()), 6);
        assert_eq!(padded, "録音  ");
    }

    #[test]
    fn pad_display_width_preserves_longer_values() {
        assert_eq!(
            pad_display_width("outside-recordings-dir", 12),
            "outside-recordings-dir"
        );
    }
}
