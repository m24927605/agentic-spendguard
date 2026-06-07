//! D15 — Manus importer binary entrypoint.
//!
//! Two modes (selected by `--mode`):
//!
//! * `fixture` — read a sanitized snapshot and print the synthesized
//!   CloudEvent envelopes to stdout (default merge gate; demo flow).
//!   In-progress sessions are filtered out (review-standards E3).
//! * `live` — pull the Manus admin REST surface directly. Requires
//!   `MANUS_API_TOKEN`. Only compiled when the `live` Cargo feature is
//!   on; the default-feature binary errors out with a clear message.
//!
//! The binary is intentionally minimal — actual ingestion happens
//! through `canonical_ingest::AppendEvents`. The demo Makefile
//! exercises this binary's emit + the demo runner inserts the rows.

use std::path::PathBuf;

use spendguard_importer_manus::{CloudEventEnvelope, FixtureLoader, PriceTable};

#[derive(Debug)]
enum Mode {
    Fixture,
    Live,
}

#[derive(Debug)]
struct CliArgs {
    mode: Mode,
    fixture: Option<PathBuf>,
    /// Override the workspace_id on every emitted record.
    workspace: Option<String>,
    /// Include in-progress sessions in the emitted envelopes
    /// (default: filtered out per E3).
    include_in_progress: bool,
}

fn parse_args() -> Result<CliArgs, String> {
    let mut args = std::env::args().skip(1);
    let mut mode: Option<Mode> = None;
    let mut fixture: Option<PathBuf> = None;
    let mut workspace: Option<String> = None;
    let mut include_in_progress = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--mode" => {
                let v = args
                    .next()
                    .ok_or_else(|| "missing value for --mode".to_string())?;
                mode = Some(match v.as_str() {
                    "fixture" => Mode::Fixture,
                    "live" => Mode::Live,
                    other => return Err(format!("unknown --mode {other}")),
                });
            }
            "--fixture" => {
                fixture = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "missing value for --fixture".to_string())?,
                ));
            }
            "--workspace" => {
                workspace = args.next();
            }
            "--include-in-progress" => {
                include_in_progress = true;
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => return Err(format!("unknown arg {other}")),
        }
    }

    Ok(CliArgs {
        mode: mode.unwrap_or(Mode::Fixture),
        fixture,
        workspace,
        include_in_progress,
    })
}

fn print_help() {
    println!(
        "spendguard importer-manus — Manus (Butterfly Effect) billing importer

USAGE:
    importer_manus --mode fixture --fixture <PATH> [--workspace <ID>] [--include-in-progress]
    importer_manus --mode live    [--workspace <ID>]   # requires MANUS_API_TOKEN

OPTIONS:
    --mode <fixture|live>     Operating mode (default: fixture)
    --fixture <PATH>          Path to sanitized manus_usage.json snapshot
    --workspace <ID>          Override the workspace_id (rare; tests only)
    --include-in-progress     Emit in_progress sessions too (default: filtered)
"
    );
}

fn run_fixture(args: &CliArgs) -> Result<(), String> {
    let path = args
        .fixture
        .as_ref()
        .ok_or_else(|| "fixture mode requires --fixture <PATH>".to_string())?;
    let loader = FixtureLoader::new(path).map_err(|e| format!("fixture load: {e}"))?;
    let prices = PriceTable::load_embedded();

    let mut envelopes: Vec<CloudEventEnvelope> = Vec::new();
    for rec in loader.records() {
        // E3: filter in_progress unless caller explicitly opts in.
        if !args.include_in_progress && !rec.status.is_terminal() {
            continue;
        }
        let mut rec = rec.clone();
        if let Some(ws) = &args.workspace {
            rec.workspace_id = ws.clone();
        }
        let env =
            spendguard_importer_manus::cloudevent_envelope::build(&rec, &prices)
                .map_err(|e| format!("envelope build: {e}"))?;
        envelopes.push(env);
    }

    let json = serde_json::to_string_pretty(&envelopes)
        .map_err(|e| format!("serialize envelopes: {e}"))?;
    println!("{json}");
    eprintln!(
        "[importer_manus] emitted {n} envelope(s) from fixture {p:?}",
        n = envelopes.len(),
        p = path,
    );
    Ok(())
}

#[cfg(feature = "live")]
fn run_live(_args: &CliArgs) -> Result<(), String> {
    Err("live mode binary scaffold not wired in this slice; \
         see services/importer_manus/src/live/* for the client API"
        .to_string())
}

#[cfg(not(feature = "live"))]
fn run_live(_args: &CliArgs) -> Result<(), String> {
    Err(
        "binary was built without the `live` Cargo feature; rebuild with \
         `cargo build -p spendguard-importer-manus --features live`"
            .to_string(),
    )
}

fn main() {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            print_help();
            std::process::exit(2);
        }
    };
    let res = match args.mode {
        Mode::Fixture => run_fixture(&args),
        Mode::Live => run_live(&args),
    };
    if let Err(e) = res {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
