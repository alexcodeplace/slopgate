//! Audit orchestrator + renderer — mirrors `src/audit/audit.mjs` (`runAudit`).
//! Assembles architecture-health sections from git facts, depcruise graph, ratchet
//! baseline, suppressions, and gate stats. Never panics.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::audit::git_facts::{
    author_shares, churn_by_file, commit_file_sets, file_at_days_ago, json_entry_history,
    AuthorShare, JsonEntryPoint,
};
use crate::audit::measures::{
    acceleration, co_change_pairs, complexity, export_ratio, fan_metrics, hotspot_score,
    is_barrel, CoChangePair, DepCruiseDependency, DepCruiseModule, FanRow,
};
use crate::checkers::depcruise::run_depcruise_json;
use crate::checkers::shared::ensure_cache_dir;
use crate::config::ResolvedConfig;
use crate::enumerate::{list_source_files, EnumerateCtx, EnumerateMode};
use crate::gate::snapshot_violations;
use crate::ratchet::load_baseline;
use crate::stats::query::{aggregate, Row};
use crate::stats::store::{project_stats_path, read_rows, ProjectStatsConfig};
use crate::suppressions::{load_suppressions, prune_stale};

const SHORT_DAYS: u32 = 30;
const CO_CHANGE_MIN_SHARED: usize = 5;
const CO_CHANGE_MIN_RATIO: f64 = 0.7;
const COMMIT_MAX_FILES: usize = 20;

/// Serialize `f64` like `JSON.stringify`: non-finite → `null`.
mod js_f64 {
    use serde::Serializer;

    pub fn serialize<S>(value: &f64, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if value.is_finite() {
            serializer.serialize_f64(*value)
        } else {
            serializer.serialize_none()
        }
    }

    #[cfg(test)]
    pub fn to_json_value(v: f64) -> serde_json::Value {
        #[derive(serde::Serialize)]
        struct Wrapper(#[serde(serialize_with = "serialize")] f64);
        serde_json::to_value(Wrapper(v)).unwrap_or(serde_json::Value::Null)
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HotspotRow {
    pub file: String,
    pub churn: u32,
    pub loc: usize,
    #[serde(rename = "fnCount")]
    pub fn_count: usize,
    pub score: f64,
    #[serde(serialize_with = "js_f64::serialize")]
    pub accel: f64,
    pub untested: bool,
    pub loc_delta: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hotspots {
    pub rows: Vec<HotspotRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeRow {
    pub dir: String,
    pub top_author: Option<String>,
    pub top_share: f64,
    pub top_commits: u32,
    pub flagged: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Knowledge {
    pub rows: Vec<KnowledgeRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Burndown {
    pub history: Vec<JsonEntryPoint>,
    pub per_day: f64,
    pub eta_days: Option<f64>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct AuditSection {
    title: String,
    lines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
struct AuditReport {
    header: String,
    sections: Vec<AuditSection>,
    notices: Vec<String>,
}

fn enumerate_ctx(config: &ResolvedConfig) -> EnumerateCtx {
    EnumerateCtx {
        repo_root: Path::new(&config.repo_root).to_path_buf(),
        roots: config
            .roots
            .iter()
            .map(Path::new)
            .map(Path::to_path_buf)
            .collect(),
        roots_rel: config.roots_rel.clone(),
        exts: config.exts.clone(),
        skip_dirs: config.skip_dirs.clone(),
    }
}

fn repo_basename(repo_root: &str) -> String {
    Path::new(repo_root)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into())
}

fn path_relative(from: &str, to: &str) -> String {
    let from_path = Path::new(from);
    let to_path = Path::new(to);
    to_path
        .strip_prefix(from_path)
        .map(|p| p.to_string_lossy().trim_start_matches('/').to_string())
        .unwrap_or_else(|_| to_path.to_string_lossy().into_owned())
}

/// Concern area = configured root + first path segment (matches diff-shape).
fn concern_area(file: &str, roots_rel: &[String]) -> Option<String> {
    let root = roots_rel
        .iter()
        .find(|r| file == *r || file.starts_with(&format!("{r}/")))?;
    let rest = if file.len() > root.len() + 1 {
        &file[root.len() + 1..]
    } else {
        ""
    };
    let seg = if rest.contains('/') {
        rest.split('/').next().unwrap_or("(root)")
    } else {
        "(root)"
    };
    Some(format!("{root}/{seg}"))
}

fn concern_areas(roots_rel: &[String]) -> Vec<String> {
    roots_rel.iter().map(|r| format!("{r}/(root)")).collect()
}

fn strip_source_ext(file: &str) -> String {
    for ext in [".tsx", ".ts", ".astro"] {
        if let Some(base) = file.strip_suffix(ext) {
            return base.to_string();
        }
    }
    file.to_string()
}

fn has_test_file(repo_root: &Path, file: &str) -> bool {
    let base = strip_source_ext(file);
    [".test.ts", ".test.tsx"]
        .iter()
        .any(|ext| repo_root.join(format!("{base}{ext}")).exists())
}

/// @returns top hotspot rows ranked by score.
pub fn build_hotspots(
    sources: &HashMap<String, String>,
    churn90: &HashMap<String, u32>,
    churn30: &HashMap<String, u32>,
    since_days: u32,
    old_source_of: Option<&dyn Fn(&str) -> Option<String>>,
    has_test: &dyn Fn(&str) -> bool,
) -> Hotspots {
    let mut rows = Vec::new();
    for (file, source) in sources {
        let churn_long = *churn90.get(file).unwrap_or(&0);
        if churn_long == 0 {
            continue;
        }
        let churn_short = *churn30.get(file).unwrap_or(&0);
        let comp = complexity(source);
        rows.push(HotspotRow {
            file: file.clone(),
            churn: churn_long,
            loc: comp.loc,
            fn_count: comp.fn_count,
            score: hotspot_score(f64::from(churn_long), &comp),
            accel: acceleration(
                f64::from(churn_short),
                f64::from(SHORT_DAYS),
                f64::from(churn_long),
                f64::from(since_days),
            ),
            untested: !has_test(file),
            loc_delta: None,
        });
    }
    rows.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut top: Vec<HotspotRow> = rows.into_iter().take(10).collect();
    if let Some(old_source_of) = old_source_of {
        for row in &mut top {
            row.loc_delta = old_source_of(&row.file).map(|old| {
                let old_loc = complexity(&old).loc;
                row.loc as i64 - old_loc as i64
            });
        }
    }
    Hotspots { rows: top }
}

/// Flags when top author share ≥ 0.8 AND commits ≥ 5.
pub fn build_knowledge(
    dir_shares: &[(String, Vec<AuthorShare>)],
    share_threshold: f64,
    min_commits: u32,
) -> Knowledge {
    let rows = dir_shares
        .iter()
        .map(|(dir, shares)| {
            let top = shares.first();
            let total: u32 = shares.iter().map(|s| s.commits).sum();
            let flagged = top.is_some_and(|t| t.share >= share_threshold && total >= min_commits);
            KnowledgeRow {
                dir: dir.clone(),
                top_author: top.map(|t| t.author.clone()),
                top_share: top.map(|t| t.share).unwrap_or(0.0),
                top_commits: top.map(|t| t.commits).unwrap_or(0),
                flagged,
            }
        })
        .collect();
    Knowledge { rows }
}

/// eta_days null when < 2 points, flat, or not decreasing.
pub fn build_burndown(history: &[JsonEntryPoint]) -> Burndown {
    if history.len() < 2 {
        return Burndown {
            history: history.to_vec(),
            per_day: 0.0,
            eta_days: None,
        };
    }
    let first = &history[0];
    let last = &history[history.len() - 1];
    let ms = iso_to_ms(&last.ts).unwrap_or(0.0) - iso_to_ms(&first.ts).unwrap_or(0.0);
    let days = ms / (1000.0 * 60.0 * 60.0 * 24.0);
    let delta = first.count as i64 - last.count as i64;
    let per_day = if days > 0.0 {
        delta as f64 / days
    } else {
        0.0
    };
    let eta_days = if delta > 0 && per_day > 0.0 && last.count > 0 {
        Some(last.count as f64 / per_day)
    } else {
        None
    };
    Burndown {
        history: history.to_vec(),
        per_day,
        eta_days,
    }
}

/// Parse ISO-8601 timestamp to Unix ms (git `%cI` / `new Date()` compatible subset).
fn iso_to_ms(ts: &str) -> Option<f64> {
    let ts = ts.trim();
    if ts.len() < 19 {
        return None;
    }
    let end = ts
        .find('+')
        .or_else(|| ts.rfind('Z'))
        .unwrap_or(ts.len())
        .min(ts.len());
    let datetime = ts[..end].split('.').next()?;
    let (date, time) = datetime.split_once('T')?;
    let mut dp = date.split('-');
    let y: i32 = dp.next()?.parse().ok()?;
    let mo: u32 = dp.next()?.parse().ok()?;
    let d: u32 = dp.next()?.parse().ok()?;
    let mut tp = time.split(':');
    let h: u32 = tp.next()?.parse().ok()?;
    let mi: u32 = tp.next()?.parse().ok()?;
    let s: u32 = tp.next()?.parse().ok()?;
    let days = civil_to_unix_days(y, mo, d)?;
    Some((days as f64 * 86400.0 + f64::from(h) * 3600.0 + f64::from(mi) * 60.0 + f64::from(s))
        * 1000.0)
}

fn civil_to_unix_days(y: i32, m: u32, d: u32) -> Option<i64> {
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let mut y = y;
    let mut m = m as i32;
    if m <= 2 {
        y -= 1;
        m += 12;
    }
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = y - era * 400;
    let doy = (153 * (m - 3) + 2) / 5 + d as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
  Some(era as i64 * 146097 + doe as i64 - 719468)
}

pub fn render_audit(header: &str, sections: &[AuditSection], notices: &[String]) -> String {
    let mut lines = vec![header.to_string(), String::new()];
    for s in sections {
        lines.push(format!("== {} ==", s.title));
        if s.lines.is_empty() {
            lines.push("(nothing to report)".into());
        } else {
            lines.extend(s.lines.clone());
        }
        lines.push(String::new());
    }
    if !notices.is_empty() {
        lines.push("-- skipped --".into());
        lines.extend(notices.iter().cloned());
    }
    lines.join("\n")
}

fn fmt_accel(v: f64) -> String {
    if v.is_infinite() && v.is_sign_positive() {
        "∞".into()
    } else if !v.is_finite() {
        v.to_string()
    } else {
        format!("{v:.2}")
    }
}

fn hotspot_lines(hs: &Hotspots) -> Vec<String> {
    hs.rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let mut extra: Vec<String> = Vec::new();
            if r.untested {
                extra.push("UNTESTED".into());
            }
            if let Some(delta) = r.loc_delta {
                if delta > 0 {
                    extra.push(format!("+{delta} loc"));
                }
            }
            let tail = if extra.is_empty() {
                String::new()
            } else {
                format!("  {}", extra.join("  "))
            };
            format!(
                "{}. {}  churn={} loc={} score={} accel={}{tail}",
                i + 1,
                r.file,
                r.churn,
                r.loc,
                r.score.round() as i64,
                fmt_accel(r.accel),
            )
        })
        .collect()
}

fn module_shape_lines(sources: &HashMap<String, String>, modules: Option<&[DepCruiseModule]>) -> Vec<String> {
    let mut lines = Vec::new();
    let fans: Vec<FanRow> = modules.map(fan_metrics).unwrap_or_default();

    for (file, source) in sources {
        let er = export_ratio(source);
        if er.total >= 5 && er.ratio >= 0.9 {
            lines.push(format!(
                "shallow export surface: {file} ({}/{total} exported)",
                er.exported,
                total = er.total
            ));
        }
        if is_barrel(source) {
            lines.push(format!("barrel: {file}"));
        }
    }

    for f in &fans {
        if f.fan_out >= 8 {
            lines.push(format!("fan-out god: {} (out={})", f.module, f.fan_out));
        }
        if f.fan_in == 1 && f.fan_out > 0 {
            lines.push(format!(
                "single-consumer: {} (in=1 out={})",
                f.module, f.fan_out
            ));
        }
    }

    lines.into_iter().take(20).collect()
}

fn co_change_lines(pairs: &[CoChangePair]) -> Vec<String> {
    pairs
        .iter()
        .take(15)
        .enumerate()
        .map(|(i, p)| {
            format!(
                "{}. {} ↔ {}  shared={} ratio={:.2}",
                i + 1,
                p.a,
                p.b,
                p.shared,
                p.ratio
            )
        })
        .collect()
}

fn knowledge_lines(k: &Knowledge) -> Vec<String> {
    let mut rows: Vec<&KnowledgeRow> = k.rows.iter().filter(|r| r.top_author.is_some()).collect();
    rows.sort_by(|a, b| {
        b.top_share
            .partial_cmp(&a.top_share)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.into_iter()
        .map(|r| {
            let flag = if r.flagged { "  ⚠ concentrated" } else { "" };
            format!(
                "{}: {} {:.0}% ({} commits){flag}",
                r.dir,
                r.top_author.as_deref().unwrap_or(""),
                r.top_share * 100.0,
                r.top_commits
            )
        })
        .collect()
}

fn ratchet_lines(bl_count: usize, current_count: usize, burndown: &Burndown) -> Vec<String> {
    let mut lines = vec![
        format!("baseline entries: {bl_count}"),
        format!("current violations (filtered): {current_count}"),
    ];
    if current_count < bl_count {
        lines.push(format!(
            "net progress: {} resolved since baseline snapshot",
            bl_count - current_count
        ));
    } else if current_count > bl_count {
        lines.push(format!(
            "regression: +{} since baseline snapshot",
            current_count - bl_count
        ));
    }
    if burndown.history.len() >= 2 {
        let eta = burndown
            .eta_days
            .map(|d| format!("~{} days", d.round() as i64))
            .unwrap_or_else(|| "n/a".into());
        lines.push(format!(
            "burn-down: {:.2} entries/day  ETA {eta}",
            burndown.per_day
        ));
    }
    lines
}

fn exemption_lines(
    sup_entries: usize,
    sup_error: Option<&str>,
    pruned_len: usize,
    ast_disable: &std::collections::HashSet<String>,
    health: Option<&Value>,
) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(err) = sup_error {
        lines.push(format!("suppressions.json malformed: {err}"));
    } else {
        lines.push(format!("active suppressions: {sup_entries}"));
    }
    if pruned_len > 0 {
        lines.push(format!("stale suppressions (dry-run): {pruned_len}"));
    }
    if !ast_disable.is_empty() {
        let joined = ast_disable.iter().cloned().collect::<Vec<_>>().join(", ");
        lines.push(format!(
            "astDisable exemptions: {joined} — still justified?"
        ));
    }
    if let Some(health) = health {
        if let Some(checkers) = health.get("checkers").and_then(|c| c.as_object()) {
            for (id, st) in checkers {
                let failures = st
                    .get("consecutiveFailures")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                if failures >= 2 {
                    lines.push(format!(
                        "CHECKER OFF: {id} ({failures} consecutive infra failures)"
                    ));
                }
            }
        }
    }
    lines
}

fn gate_effectiveness_lines(stats: &crate::stats::query::AggregateResult) -> Vec<String> {
    if stats.total == 0 {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "{} incident(s) stopped (gate effectiveness)",
        stats.total
    )];
    for g in stats.groups.iter().take(10) {
        lines.push(format!("  {}: {}", g.key, g.count));
    }
    lines
}

fn parse_depcruise_modules(data: &Value) -> Option<Vec<DepCruiseModule>> {
    let modules = data.get("modules")?.as_array()?;
    Some(
        modules
            .iter()
            .filter_map(|m| {
                let source = m.get("source")?.as_str()?.to_string();
                let dependencies = m
                    .get("dependencies")
                    .and_then(|d| d.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|dep| {
                                Some(DepCruiseDependency {
                                    resolved: dep.get("resolved")?.as_str()?.to_string(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                Some(DepCruiseModule {
                    source,
                    dependencies,
                })
            })
            .collect(),
    )
}

fn rows_from_jsonl(values: &[Value]) -> Vec<Row> {
    values
        .iter()
        .filter_map(|v| serde_json::from_value(v.clone()).ok())
        .collect()
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown error".into()
    }
}

/// Run the full audit report. `since_days` defaults to 90 when callers pass that value.
pub fn run_audit(config: &ResolvedConfig, since_days: u32, json: bool) -> String {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_audit_inner(config, since_days, json)
    })) {
        Ok(out) => out,
        Err(payload) => {
            let project = repo_basename(&config.repo_root);
            let header = format!("SLOPGATE AUDIT — {project} — window {since_days}d");
            let msg = panic_message(payload);
            let report = AuditReport {
                header: header.clone(),
                sections: vec![],
                notices: vec![format!("audit error: {msg}")],
            };
            if json {
                serde_json::to_string(&report).unwrap_or_else(|_| "{}".into())
            } else {
                render_audit(&header, &report.sections, &report.notices)
            }
        }
    }
}

fn run_audit_inner(config: &ResolvedConfig, since_days: u32, json: bool) -> String {
    let mut notices: Vec<String> = Vec::new();
    let mut sections: Vec<AuditSection> = Vec::new();
    let project = repo_basename(&config.repo_root);
    let header = format!("SLOPGATE AUDIT — {project} — window {since_days}d");
    let repo_root = Path::new(&config.repo_root);

    let outer = (|| -> Result<(), String> {
        let ctx = enumerate_ctx(config);
        let files = list_source_files(&ctx, EnumerateMode::Walk);
        let mut sources: HashMap<String, String> = HashMap::new();
        for f in &files {
            let path = repo_root.join(f);
            if let Ok(content) = fs::read_to_string(&path) {
                sources.insert(f.clone(), content);
            }
        }

        // Hotspots (git)
        {
            let churn90 = churn_by_file(repo_root, since_days);
            let churn30 = churn_by_file(repo_root, SHORT_DAYS);
            if churn90.is_empty() {
                notices.push("hotspots skipped (no git history)".into());
            } else {
                let hs = build_hotspots(
                    &sources,
                    &churn90,
                    &churn30,
                    since_days,
                    Some(&|f| file_at_days_ago(repo_root, f, since_days)),
                    &|f| has_test_file(repo_root, f),
                );
                sections.push(AuditSection {
                    title: "Hotspots (churn x size)".into(),
                    lines: hotspot_lines(&hs),
                });
            }
        }

        // Module shape (file metrics always; graph metrics when depcruise available)
        {
            let dep_cfg = config.checkers.get("depcruise");
            let mut modules: Option<Vec<DepCruiseModule>> = None;
            if dep_cfg.is_none() {
                notices.push("module graph skipped (no depcruise config)".into());
            } else {
                let result = run_depcruise_json(config, dep_cfg.unwrap());
                if let Some(data) = result.data.as_ref().and_then(parse_depcruise_modules) {
                    modules = Some(data);
                } else {
                    let err = result
                        .errors
                        .first()
                        .map(String::as_str)
                        .unwrap_or("no depcruise output");
                    notices.push(format!("module graph skipped ({err})"));
                }
            }
            sections.push(AuditSection {
                title: "Module shape".into(),
                lines: module_shape_lines(&sources, modules.as_deref()),
            });
        }

        // Co-change coupling (git)
        {
            let sets = commit_file_sets(repo_root, since_days, COMMIT_MAX_FILES);
            if sets.is_empty() {
                notices.push("co-change skipped (no git history)".into());
            } else {
                let roots = config.roots_rel.clone();
                let pairs = co_change_pairs(
                    &sets,
                    |f| concern_area(f, &roots),
                    CO_CHANGE_MIN_SHARED,
                    CO_CHANGE_MIN_RATIO,
                );
                sections.push(AuditSection {
                    title: "Co-change coupling".into(),
                    lines: co_change_lines(&pairs),
                });
            }
        }

        // Knowledge concentration (git)
        {
            let mut areas: BTreeSet<String> = BTreeSet::new();
            for f in &files {
                if let Some(a) = concern_area(f, &config.roots_rel) {
                    areas.insert(a);
                }
            }
            for a in concern_areas(&config.roots_rel) {
                areas.insert(a);
            }
            let dir_shares: Vec<(String, Vec<AuthorShare>)> = areas
                .into_iter()
                .map(|dir| {
                    let shares = author_shares(repo_root, since_days, &dir);
                    (dir, shares)
                })
                .collect();
            let k = build_knowledge(&dir_shares, 0.8, 5);
            sections.push(AuditSection {
                title: "Knowledge concentration".into(),
                lines: knowledge_lines(&k),
            });
        }

        // Ratchet progress + burn-down
        {
            let bl = load_baseline(Path::new(&config.baseline_path));
            if bl.missing {
                notices.push("ratchet progress skipped (no valid baseline.json)".into());
            } else if let Some(err) = &bl.error {
                notices.push(format!("ratchet progress skipped (baseline malformed: {err})"));
            } else {
                let rel_baseline = path_relative(&config.repo_root, &config.baseline_path);
                let hist = json_entry_history(repo_root, &rel_baseline, "entries");
                let burndown = build_burndown(&hist);
                let current_count = snapshot_violations(config).len();
                sections.push(AuditSection {
                    title: "Ratchet progress + burn-down".into(),
                    lines: ratchet_lines(bl.entries.len(), current_count, &burndown),
                });
            }
        }

        // Exemptions & checker health
        {
            let sup = load_suppressions(Path::new(&config.suppressions_path));
            let pruned = prune_stale(repo_root, Path::new(&config.suppressions_path), true);
            let health_path = ensure_cache_dir(Path::new(&config.config_dir))
                .ok()
                .map(|d| d.join("checker-health.json"))
                .filter(|p| p.exists());
            let health: Option<Value> = health_path.and_then(|p| {
                fs::read_to_string(p)
                    .ok()
                    .and_then(|s| serde_json::from_str(&s).ok())
            });
            sections.push(AuditSection {
                title: "Exemptions & checker health".into(),
                lines: exemption_lines(
                    sup.entries.len(),
                    sup.error.as_deref(),
                    pruned.pruned.len(),
                    &config.ast_disable,
                    health.as_ref(),
                ),
            });
        }

        // Gate effectiveness (stats.jsonl)
        {
            let path = project_stats_path(config);
            let raw = read_rows(&path);
            if raw.is_empty() {
                notices.push("gate effectiveness skipped (no stats.jsonl)".into());
            } else {
                let rows = rows_from_jsonl(&raw);
                let stats = aggregate(&rows, None, None).unwrap_or(
                    crate::stats::query::AggregateResult {
                        total: 0,
                        by: "rule".into(),
                        last_seen: None,
                        first_seen: None,
                        groups: vec![],
                    },
                );
                sections.push(AuditSection {
                    title: "Gate effectiveness".into(),
                    lines: gate_effectiveness_lines(&stats),
                });
            }
        }

        Ok(())
    })();

    if let Err(e) = outer {
        notices.push(format!("audit error: {e}"));
    }

    let report = AuditReport {
        header: header.clone(),
        sections,
        notices,
    };
    if json {
        serde_json::to_string(&report).unwrap_or_else(|_| "{}".into())
    } else {
        render_audit(&header, &report.sections, &report.notices)
    }
}

impl ProjectStatsConfig for ResolvedConfig {
    fn config_dir(&self) -> &Path {
        Path::new(&self.config_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::resolve_config_str;
    use serde_json::json;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    fn run_git(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("spawn git");
        assert!(status.success(), "git {:?} failed", args);
    }

    fn init_repo(repo: &Path) {
        run_git(repo, &["init", "-b", "main"]);
        run_git(repo, &["config", "user.email", "audit@example.com"]);
        run_git(repo, &["config", "user.name", "Audit User"]);
    }

    fn write_commit(repo: &Path, path: &str, content: &str, msg: &str) {
        let file_path = repo.join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&file_path, content).unwrap();
        run_git(repo, &["add", path]);
        run_git(repo, &["commit", "-m", msg]);
    }

    fn write_commit_dated(repo: &Path, path: &str, content: &str, msg: &str, date: &str) {
        let file_path = repo.join(path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&file_path, content).unwrap();
        run_git(repo, &["add", path]);
        let status = Command::new("git")
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date)
            .args(["commit", "-m", msg])
            .current_dir(repo)
            .status()
            .expect("dated commit");
        assert!(status.success());
    }

    fn test_config(root: &Path) -> ResolvedConfig {
        let toml = fs::read_to_string(format!(
            "{}/tests/fixtures/config.toml",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let config_dir = root.join(".slopgate");
        fs::create_dir_all(&config_dir).unwrap();
        let mut config = resolve_config_str(&toml).unwrap();
        config.repo_root = root.to_string_lossy().into_owned();
        config.config_dir = config_dir.to_string_lossy().into_owned();
        config.roots = config
            .roots_rel
            .iter()
            .map(|r| root.join(r).to_string_lossy().into_owned())
            .collect();
        config.baseline_path = config_dir
            .join("baseline.json")
            .to_string_lossy()
            .into_owned();
        config.suppressions_path = config_dir
            .join("suppressions.json")
            .to_string_lossy()
            .into_owned();
        fs::write(
            &config.suppressions_path,
            r#"{"version":1,"entries":[]}"#,
        )
        .unwrap();
        fs::write(
            &config.baseline_path,
            r#"{"version":1,"entries":{}}"#,
        )
        .unwrap();
        config
    }

    fn sample_audit_repo() -> (TempDir, ResolvedConfig) {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        init_repo(root);
        fs::create_dir_all(root.join("src")).unwrap();
        write_commit(
            root,
            "src/app.ts",
            "function f() {}\nfunction g() {}\nexport const x = 1;\n",
            "add app",
        );
        write_commit(
            root,
            "src/app.ts",
            "function f() {}\nfunction g() {}\nfunction h() {}\nexport const x = 1;\n",
            "grow app",
        );
        write_commit(root, "src/util.ts", "export const u = 1;\n", "add util");
        let config = test_config(root);
        (dir, config)
    }

    #[test]
    fn infinity_serializes_as_null_in_json() {
        let v = js_f64::to_json_value(f64::INFINITY);
        assert_eq!(v, json!(null));
        let v = js_f64::to_json_value(f64::NEG_INFINITY);
        assert_eq!(v, json!(null));
        let finite = js_f64::to_json_value(1.5);
        assert_eq!(finite, json!(1.5));
    }

    #[test]
    fn fmt_accel_renders_infinity_as_symbol() {
        assert_eq!(fmt_accel(f64::INFINITY), "∞");
        assert_eq!(fmt_accel(1.234), "1.23");
    }

    #[test]
    fn build_burndown_eta_when_decreasing() {
        let hist = vec![
            JsonEntryPoint {
                ts: "2024-01-01T00:00:00Z".into(),
                count: 10,
            },
            JsonEntryPoint {
                ts: "2024-01-11T00:00:00Z".into(),
                count: 5,
            },
        ];
        let b = build_burndown(&hist);
        assert!((b.per_day - 0.5).abs() < 0.01);
        assert!((b.eta_days.unwrap() - 10.0).abs() < 0.5);
    }

    #[test]
    fn run_audit_text_contains_header_and_sections() {
        let (_dir, config) = sample_audit_repo();
        let out = run_audit(&config, 90, false);
        assert!(out.starts_with("SLOPGATE AUDIT — "));
        assert!(out.contains("window 90d"));
        assert!(out.contains("== Hotspots (churn x size) =="));
        assert!(out.contains("== Module shape =="));
        assert!(out.contains("== Knowledge concentration =="));
        assert!(out.contains("== Exemptions & checker health =="));
        assert!(out.contains("active suppressions: 0"));
    }

    #[test]
    fn run_audit_json_matches_text_header() {
        let (_dir, config) = sample_audit_repo();
        let text = run_audit(&config, 90, false);
        let json_out = run_audit(&config, 90, true);
        let parsed: AuditReport = serde_json::from_str(&json_out).expect("valid audit json");
        assert!(text.starts_with(&parsed.header));
        assert!(!parsed.sections.is_empty());
        let titles: Vec<&str> = parsed.sections.iter().map(|s| s.title.as_str()).collect();
        assert!(titles.contains(&"Hotspots (churn x size)"));
        assert!(titles.contains(&"Module shape"));
    }

    #[test]
    fn run_audit_since_days_filters_old_commits() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        init_repo(root);
        fs::create_dir_all(root.join("src")).unwrap();
        write_commit_dated(
            root,
            "src/old.ts",
            "function old() {}\n",
            "ancient",
            "2020-01-01T00:00:00",
        );
        write_commit(root, "src/new.ts", "function newFn() {}\n", "recent");
        let config = test_config(root);

        let recent_only = run_audit(&config, 30, false);
        assert!(recent_only.contains("src/new.ts"));
        assert!(!recent_only.contains("src/old.ts"));

        let long_window = run_audit(&config, 3650, false);
        assert!(long_window.contains("src/old.ts"));
    }

    #[test]
    fn run_audit_non_git_dir_never_panics() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.ts"), "export const a = 1;\n").unwrap();
        let config = test_config(root);
        let out = run_audit(&config, 90, false);
        assert!(out.contains("SLOPGATE AUDIT"));
        assert!(out.contains("hotspots skipped (no git history)"));
    }

    #[test]
    fn hotspot_row_accel_infinity_in_structured_json() {
        let row = HotspotRow {
            file: "src/a.ts".into(),
            churn: 5,
            loc: 10,
            fn_count: 2,
            score: 100.0,
            accel: f64::INFINITY,
            untested: true,
            loc_delta: None,
        };
        let v = serde_json::to_value(&row).unwrap();
        assert_eq!(v["accel"], json!(null));
    }
}
