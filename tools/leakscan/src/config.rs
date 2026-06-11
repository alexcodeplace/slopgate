//! Scan configuration: which files count as presentation layer, which modules /
//! call shapes count as direct I/O. Defaults are sensible; everything is
//! overridable from a JSON config so projects pin their own layout.

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Deserialize;

/// Raw config as read from JSON (all fields optional → merged onto defaults).
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RawConfig {
    /// Globs whose files are treated as the presentation layer (components, pages, UI).
    pub presentation_globs: Vec<String>,
    /// Globs that opt a file back OUT of the presentation layer (the allowed seam:
    /// service modules, api clients, data-access). Checked after presentation_globs.
    pub exempt_globs: Vec<String>,
    /// Bare module specifiers that are data/transport layers — importing them from a
    /// presentation file is a leak (drizzle, prisma, pg, axios, ...).
    pub banned_modules: Vec<String>,
    /// Member-call method names that signal a raw DB call (`db.query(...)`).
    pub db_methods: Vec<String>,
    /// Tagged-template tag names that signal an inline query (`sql`...``).
    pub query_tags: Vec<String>,
    /// Global call identifiers that are direct transport (`fetch`, `XMLHttpRequest`).
    /// Suppressed when the name is locally bound (import/const/param) in the file.
    pub global_calls: Vec<String>,
}

/// Compiled, ready-to-match configuration.
pub struct Config {
    pub presentation: GlobSet,
    pub exempt: GlobSet,
    pub banned_modules: Vec<String>,
    pub db_methods: Vec<String>,
    pub query_tags: Vec<String>,
    pub global_calls: Vec<String>,
}

fn defaults() -> RawConfig {
    RawConfig {
        presentation_globs: svec(&[
            "**/components/**",
            "**/ui/**",
            "**/pages/**",
            "**/app/**",
            "**/views/**",
            "**/*.tsx",
            "**/*.jsx",
        ]),
        exempt_globs: svec(&[
            "**/services/**",
            "**/service/**",
            "**/lib/api/**",
            "**/api-client/**",
            "**/data/**",
            "**/data-access/**",
            "**/repositories/**",
            "**/*.test.*",
            "**/*.spec.*",
            "**/*.stories.*",
            "**/node_modules/**",
        ]),
        banned_modules: svec(&[
            "pg",
            "mysql",
            "mysql2",
            "better-sqlite3",
            "drizzle-orm",
            "@prisma/client",
            "prisma",
            "knex",
            "typeorm",
            "mongoose",
            "@supabase/supabase-js",
            "axios",
            "node-fetch",
            "got",
            "ky",
        ]),
        db_methods: svec(&["query", "execute", "raw", "$queryRaw", "$executeRaw"]),
        query_tags: svec(&["sql"]),
        global_calls: svec(&["fetch", "XMLHttpRequest"]),
    }
}

impl Config {
    /// Build from optional user JSON, merged onto defaults (user list replaces default
    /// list when non-empty — keeps semantics obvious: empty == "use default").
    pub fn load(user: Option<RawConfig>) -> Result<Self, String> {
        let d = defaults();
        let u = user.unwrap_or_default();
        let pick = |a: Vec<String>, b: Vec<String>| if a.is_empty() { b } else { a };

        Ok(Config {
            presentation: build_set(&pick(u.presentation_globs, d.presentation_globs))?,
            exempt: build_set(&pick(u.exempt_globs, d.exempt_globs))?,
            banned_modules: pick(u.banned_modules, d.banned_modules),
            db_methods: pick(u.db_methods, d.db_methods),
            query_tags: pick(u.query_tags, d.query_tags),
            global_calls: pick(u.global_calls, d.global_calls),
        })
    }

    /// A file is in scope when it matches a presentation glob and is NOT exempt.
    pub fn is_presentation(&self, path: &str) -> bool {
        self.presentation.is_match(path) && !self.exempt.is_match(path)
    }
}

fn build_set(globs: &[String]) -> Result<GlobSet, String> {
    let mut b = GlobSetBuilder::new();
    for g in globs {
        b.add(Glob::new(g).map_err(|e| format!("bad glob {g:?}: {e}"))?);
    }
    b.build().map_err(|e| format!("globset build: {e}"))
}

fn svec(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}
