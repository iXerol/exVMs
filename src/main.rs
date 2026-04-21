use std::io::{self, Write};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use exvms::db;
use exvms::export::{self, ExportReport, SelectionMode};
use exvms::storage;
use exvms::time;

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
        #[arg(long)]
        all: bool,
        /// Parallel export job count. Defaults to min(4, available parallelism).
        #[arg(long)]
        jobs: Option<usize>,
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
        } => export_cmd(output, ids, all, jobs),
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
    writeln!(
        out,
        "{:<6} {:<20} {:<10} {:<23} {:<18} {:<12} {:<20} {:<28} title",
        "id", "recorded_at", "duration", "availability", "store", "path", "folder", "db_path"
    )?;

    for row in rows {
        let availability = match export::classify_availability(&store.recordings_dir, &row)? {
            export::Availability::Local => "local".to_string(),
            export::Availability::NeedsDownload(_) => "needs-download".to_string(),
            export::Availability::OutsideRecordingsDir(_) => "outside-recordings-dir".to_string(),
        };
        writeln!(
            out,
            "{:<6} {:<20} {:<10} {:<23} {:<18} {:<12} {:<20} {:<28} {}",
            row.id,
            time::format_apple_local(row.recorded_at),
            time::format_duration(row.duration),
            availability,
            row.store_kind.as_str(),
            row.path_kind.as_str(),
            row.folder_name.as_deref().unwrap_or(""),
            row.db_path,
            row.title
        )?;
    }

    Ok(())
}

fn export_cmd(output: PathBuf, ids: Vec<i64>, all: bool, jobs: Option<usize>) -> exvms::Result<()> {
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
    let (planned, skipped) =
        export::plan_exports(&store.recordings_dir, &output, &rows, selected, mode)?;
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
