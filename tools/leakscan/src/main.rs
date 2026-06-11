//! leakscan — leaky-abstraction detector.
//!
//! Walks the given roots, parses every JS/TS/JSX/TSX file that classifies as the
//! presentation layer, and reports direct DB / external-API I/O. Emits a single
//! JSON document on stdout so a slopgate checker adapter can consume it.
//!
//! Usage:
//!   leakscan [--config <file.json>] <root> [<root> ...]
//!
//! Exit code: 0 always (a gate decides pass/fail from the JSON; the scanner only
//! reports). Non-zero only on a usage error that prevented scanning.

mod config;
mod detect;

use std::path::Path;
use std::process::ExitCode;

use oxc::allocator::Allocator;
use oxc::parser::Parser;
use oxc::span::SourceType;
use serde::Serialize;
use walkdir::WalkDir;

use config::{Config, RawConfig};
use detect::{BindingCollector, Detector, Finding, LineIndex};
use oxc::ast_visit::Visit;

#[derive(Serialize)]
struct FileViolation {
    file: String,
    line: u32,
    col: u32,
    rule: String,
    severity: String,
    message: String,
    snippet: String,
}

#[derive(Serialize)]
struct Report {
    violations: Vec<FileViolation>,
    scanned: usize,
    errors: Vec<String>,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut config_path: Option<String> = None;
    let mut roots: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--config" | "-c" => {
                i += 1;
                let Some(p) = args.get(i) else {
                    eprintln!("leakscan: --config needs a path");
                    return ExitCode::from(2);
                };
                config_path = Some(p.clone());
            }
            "--help" | "-h" => {
                eprintln!("usage: leakscan [--config <file.json>] <root> [<root> ...]");
                return ExitCode::SUCCESS;
            }
            other => roots.push(other.to_string()),
        }
        i += 1;
    }

    if roots.is_empty() {
        roots.push(".".to_string());
    }

    let user_cfg = match config_path {
        Some(p) => match std::fs::read_to_string(&p) {
            Ok(s) => match serde_json::from_str::<RawConfig>(&s) {
                Ok(c) => Some(c),
                Err(e) => {
                    print_report(Report { violations: vec![], scanned: 0, errors: vec![format!("config parse {p}: {e}")] });
                    return ExitCode::SUCCESS;
                }
            },
            Err(e) => {
                print_report(Report { violations: vec![], scanned: 0, errors: vec![format!("config read {p}: {e}")] });
                return ExitCode::SUCCESS;
            }
        },
        None => None,
    };

    let cfg = match Config::load(user_cfg) {
        Ok(c) => c,
        Err(e) => {
            print_report(Report { violations: vec![], scanned: 0, errors: vec![e] });
            return ExitCode::SUCCESS;
        }
    };

    let mut violations = Vec::new();
    let mut errors = Vec::new();
    let mut scanned = 0usize;

    for root in &roots {
        for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if !is_source(path) {
                continue;
            }
            let rel = path.to_string_lossy().replace('\\', "/");
            if !cfg.is_presentation(&rel) {
                continue;
            }
            match std::fs::read_to_string(path) {
                Ok(src) => {
                    scanned += 1;
                    scan_file(&cfg, &rel, &src, &mut violations);
                }
                Err(e) => errors.push(format!("read {rel}: {e}")),
            }
        }
    }

    violations.sort_by(|a, b| {
        a.file.cmp(&b.file).then(a.line.cmp(&b.line)).then(a.col.cmp(&b.col))
    });

    print_report(Report { violations, scanned, errors });
    ExitCode::SUCCESS
}

fn scan_file(cfg: &Config, rel: &str, src: &str, out: &mut Vec<FileViolation>) {
    let alloc = Allocator::default();
    let source_type = SourceType::from_path(Path::new(rel)).unwrap_or_default();
    let parsed = Parser::new(&alloc, src, source_type).parse();
    // Parser is error-tolerant; a partial AST is still worth scanning, so we don't bail on parsed.errors.

    let mut collector = BindingCollector::default();
    collector.visit_program(&parsed.program);

    let line_index = LineIndex::new(src);
    let mut detector = Detector::new(cfg, &collector.names, &line_index);
    detector.visit_program(&parsed.program);

    for f in detector.findings.drain(..) {
        let Finding { line, col, rule, severity, message, snippet } = f;
        out.push(FileViolation { file: rel.to_string(), line, col, rule, severity, message, snippet });
    }
}

fn is_source(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "mts" | "cts")
    )
}

fn print_report(report: Report) {
    match serde_json::to_string(&report) {
        Ok(s) => println!("{s}"),
        Err(e) => {
            // Last-resort: emit a minimal valid envelope so the adapter never chokes.
            println!("{{\"violations\":[],\"scanned\":0,\"errors\":[\"serialize: {e}\"]}}");
        }
    }
}
