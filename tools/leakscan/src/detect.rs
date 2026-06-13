//! AST passes over one presentation-layer file.
//!
//! Pass 1 (`BindingCollector`) records every name bound in the file — imports,
//! const/let/var, function names, params. Pass 2 (`Detector`) flags direct I/O,
//! suppressing the global-call rule when the call name is locally bound (e.g. a
//! `const fetch = serviceWrapper` or `import { fetch } from './api'` seam).

use oxc::ast::ast::*;
use oxc::ast_visit::{walk, Visit};
use oxc::span::Span;
use std::collections::HashSet;

use crate::config::Config;

#[derive(Debug, serde::Serialize)]
pub struct Finding {
    pub line: u32,
    pub col: u32,
    pub rule: String,
    pub severity: String,
    pub message: String,
    pub snippet: String,
}

/// Pass 1: gather all locally-bound identifier names.
#[derive(Default)]
pub struct BindingCollector {
    pub names: HashSet<String>,
}

impl<'a> Visit<'a> for BindingCollector {
    fn visit_binding_identifier(&mut self, it: &BindingIdentifier<'a>) {
        self.names.insert(it.name.to_string());
    }
    // Import locals are BindingIdentifiers under specifiers; the default walk reaches them.
}

/// Pass 2: detect direct DB / external-API usage.
pub struct Detector<'c> {
    cfg: &'c Config,
    bound: &'c HashSet<String>,
    line_index: &'c LineIndex<'c>,
    pub findings: Vec<Finding>,
}

impl<'c> Detector<'c> {
    pub fn new(cfg: &'c Config, bound: &'c HashSet<String>, line_index: &'c LineIndex<'c>) -> Self {
        Detector {
            cfg,
            bound,
            line_index,
            findings: Vec::new(),
        }
    }

    fn push(&mut self, span: Span, rule: &str, severity: &str, message: String) {
        let (line, col) = self.line_index.locate(span.start);
        self.findings.push(Finding {
            line,
            col,
            rule: rule.to_string(),
            severity: severity.to_string(),
            message,
            snippet: self.line_index.line_text(line).trim().to_string(),
        });
    }
}

impl<'a, 'c> Visit<'a> for Detector<'c> {
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        let src = it.source.value.as_str();
        if self.cfg.banned_modules.iter().any(|m| m == src) {
            self.push(
                it.span,
                "banned-import-in-component",
                "high",
                format!("presentation file imports data/transport module `{src}` — route through a service layer"),
            );
        }
        walk::walk_import_declaration(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        match &it.callee {
            // Bare global call: fetch(...), XMLHttpRequest(...). Suppress if locally bound.
            Expression::Identifier(id) => {
                let name = id.name.as_str();
                if self.cfg.global_calls.iter().any(|g| g == name) && !self.bound.contains(name) {
                    self.push(
                        it.span,
                        "raw-global-io-in-component",
                        "high",
                        format!("direct `{name}(...)` in presentation file — wrap transport in a service/API client"),
                    );
                }
            }
            // Member call: db.query(...), prisma.$queryRaw(...).
            Expression::StaticMemberExpression(m) => {
                let method = m.property.name.as_str();
                if self.cfg.db_methods.iter().any(|d| d == method) {
                    self.push(
                        it.span,
                        "raw-db-call-in-component",
                        "high",
                        format!("direct `.{method}(...)` data call in presentation file — move to a repository/service"),
                    );
                }
            }
            _ => {}
        }
        walk::walk_call_expression(self, it);
    }

    fn visit_tagged_template_expression(&mut self, it: &TaggedTemplateExpression<'a>) {
        if let Expression::Identifier(tag) = &it.tag {
            let name = tag.name.as_str();
            if self.cfg.query_tags.iter().any(|q| q == name) {
                self.push(
                    it.span,
                    "inline-query-in-component",
                    "medium",
                    format!("inline `{name}`...`` query in presentation file — keep SQL in the data layer"),
                );
            }
        }
        walk::walk_tagged_template_expression(self, it);
    }
}

/// Byte-offset → (1-based line, 1-based col) and line-text lookup over a source string.
pub struct LineIndex<'s> {
    src: &'s str,
    /// Byte offset at the start of each line.
    starts: Vec<u32>,
}

impl<'s> LineIndex<'s> {
    pub fn new(src: &'s str) -> Self {
        let mut starts = vec![0u32];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push((i + 1) as u32);
            }
        }
        LineIndex { src, starts }
    }

    pub fn locate(&self, offset: u32) -> (u32, u32) {
        // Largest line start <= offset.
        let line = match self.starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let col = offset - self.starts[line] + 1;
        ((line as u32) + 1, col)
    }

    pub fn line_text(&self, line_1based: u32) -> &str {
        let idx = (line_1based - 1) as usize;
        let start = self.starts[idx] as usize;
        let end = self
            .starts
            .get(idx + 1)
            .map(|&s| s as usize)
            .unwrap_or(self.src.len());
        self.src[start..end].trim_end_matches('\n')
    }
}
