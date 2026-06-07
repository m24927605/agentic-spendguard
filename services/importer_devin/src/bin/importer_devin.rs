//! D14 — Devin importer binary entrypoint.
//!
//! Two modes (selected by `--mode`):
//!
//! * `fixture` — read a sanitized snapshot and print the synthesized
//!   CloudEvent envelopes to stdout (default merge gate; demo flow).
//! * `live` — pull the Devin Team API directly. Requires
//!   `DEVIN_API_TOKEN`. Only compiled when the `live` Cargo feature is
//!   on; the default-feature binary errors out with a clear message.
//!
//! The binary is intentionally minimal — actual ingestion happens
//! through `canonical_ingest::AppendEvents`, which the demo Makefile
//! exercises via psql. This binary's job is "emit envelopes"; the
//! gRPC handoff is the library API.

use std::path::PathBuf;

use spendguard_importer_devin::{AcuPriceTable, CloudEventEnvelope, FixtureLoader};

#[derive(Debug)]
enum Mode {
    Fixture,
    Live,
}

#[derive(Debug)]
struct CliArgs {
    mode: Mode,
    fixture: Option<PathBuf>,
    tenant: Option<String>,
    budget: Option<String>,
}

fn parse_args() -> Result<CliArgs, String> {
    let mut args = std::env::args().skip(1);
    let mut mode: Option<Mode> = None;
    let mut fixture: Option<PathBuf> = None;
    let mut tenant: Option<String> = None;
    let mut budget: Option<String> = None;

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
            "--tenant" => {
                tenant = args.next();
            }
            "--budget" => {
                budget = args.next();
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
        tenant,
        budget,
    })
}

fn print_help() {
    println!(
        "spendguard importer-devin — Devin (Cognition Labs) billing importer

USAGE:
    importer_devin --mode fixture --fixture <PATH> [--tenant <ID>] [--budget <ID>]
    importer_devin --mode live    [--tenant <ID>] [--budget <ID>]   # requires DEVIN_API_TOKEN

OPTIONS:
    --mode <fixture|live>   Operating mode (default: fixture)
    --fixture <PATH>        Path to sanitized devin_usage.json snapshot
    --tenant <ID>           Override the tenant_id from the fixture
    --budget <ID>           Override the budget_id from the fixture
"
    );
}

fn run_fixture(args: &CliArgs) -> Result<(), String> {
    let path = args
        .fixture
        .as_ref()
        .ok_or_else(|| "fixture mode requires --fixture <PATH>".to_string())?;
    let loader = FixtureLoader::new(path).map_err(|e| format!("fixture load: {e}"))?;
    let prices = AcuPriceTable::load_from_embedded();

    let mut envelopes: Vec<CloudEventEnvelope> = Vec::new();
    for rec in loader.records() {
        // Apply --tenant / --budget overrides if any.
        let mut rec = rec.clone();
        if let Some(t) = &args.tenant {
            rec.tenant_id = t.clone();
        }
        if let Some(b) = &args.budget {
            rec.budget_id = b.clone();
        }
        let env = spendguard_importer_devin::cloudevent_envelope::build(&rec, &prices)
            .map_err(|e| format!("envelope build: {e}"))?;
        envelopes.push(env);
    }

    let json = serde_json::to_string_pretty(&envelopes)
        .map_err(|e| format!("serialize envelopes: {e}"))?;
    println!("{json}");
    eprintln!(
        "[importer_devin] emitted {n} envelope(s) from fixture {p:?}",
        n = envelopes.len(),
        p = path,
    );
    Ok(())
}

#[cfg(feature = "live")]
fn run_live(_args: &CliArgs) -> Result<(), String> {
    // The default-feature build doesn't link this branch — the
    // workspace `cargo build` still succeeds.
    Err("live mode binary scaffold not wired in this slice; \
         see services/importer_devin/src/live/* for the client API"
        .to_string())
}

#[cfg(not(feature = "live"))]
fn run_live(_args: &CliArgs) -> Result<(), String> {
    Err(
        "binary was built without the `live` Cargo feature; rebuild with \
         `cargo build -p spendguard-importer-devin --features live`"
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
