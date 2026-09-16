//! Executable architecture contract, SG-GOV-001. This tool reads source as data;
//! it never executes candidate code. Human-owned policy changes still require
//! independent protected-branch review. No source checker can constrain an admin
//! who can replace both the checker and its protection settings.
use quote::ToTokens;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use syn::visit::{self, Visit};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    pub version: u32,
    pub crates: BTreeMap<String, CratePolicy>,
    pub forbidden_core_literals: BTreeSet<String>,
    pub process_owner: String,
    pub specification: String,
    pub traceability: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CratePolicy {
    pub path: String,
    pub local_dependencies: BTreeSet<String>,
    pub external_dependencies: BTreeSet<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceMap {
    pub version: u32,
    pub requirements: BTreeMap<String, Evidence>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Evidence {
    pub implementation: Vec<String>,
    pub tests: Vec<String>,
}

pub fn inspect(root: &Path) -> Result<Vec<String>, String> {
    let root = root.canonicalize().map_err(|error| error.to_string())?;
    let policy: Policy = serde_json::from_str(&read_owned(&root, "docs/architecture/policy.json")?)
        .map_err(|error| format!("architecture policy: {error}"))?;
    if policy.version != 1 {
        return Err("unsupported architecture policy version".into());
    }
    let mut errors = Vec::new();
    let workspace: toml::Value = toml::from_str(&read_owned(&root, "Cargo.toml")?)
        .map_err(|error| format!("workspace manifest: {error}"))?;
    let members = workspace
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(toml::Value::as_array)
        .ok_or("workspace members must be explicit paths")?;
    let declared: BTreeSet<_> = members
        .iter()
        .filter_map(toml::Value::as_str)
        .map(str::to_string)
        .collect();
    let expected: BTreeSet<_> = policy
        .crates
        .values()
        .map(|entry| entry.path.clone())
        .collect();
    if declared != expected {
        errors.push(format!(
            "SG-ARCH-001: workspace membership drift: declared {declared:?}, approved {expected:?}"
        ));
    }
    if workspace.get("patch").is_some() || workspace.get("replace").is_some() {
        errors.push("SG-ARCH-001: workspace dependency replacement requires an explicit reviewed policy extension".into());
    }
    for (name, entry) in &policy.crates {
        let manifest_path = format!("{}/Cargo.toml", entry.path);
        let manifest: toml::Value = match toml::from_str(&read_owned(&root, &manifest_path)?) {
            Ok(manifest) => manifest,
            Err(error) => {
                errors.push(format!("{manifest_path}: {error}"));
                continue;
            }
        };
        if manifest
            .get("package")
            .and_then(|v| v.get("name"))
            .and_then(toml::Value::as_str)
            != Some(name.as_str())
        {
            errors.push(format!(
                "{manifest_path}: package identity does not match policy"
            ));
        }
        let crate_dir = root.join(&entry.path);
        let mut dependencies = Vec::new();
        collect_dependencies(&manifest, &mut dependencies);
        for (alias, local_value) in dependencies {
            let inherited =
                local_value.get("workspace").and_then(toml::Value::as_bool) == Some(true);
            let value = if inherited {
                workspace
                    .get("workspace")
                    .and_then(|workspace| workspace.get("dependencies"))
                    .and_then(|dependencies| dependencies.get(&alias))
                    .cloned()
                    .ok_or_else(|| {
                        format!("{manifest_path}: inherited dependency {alias} is missing")
                    })?
            } else {
                local_value
            };
            let package = value
                .get("package")
                .and_then(toml::Value::as_str)
                .unwrap_or(&alias);
            let approved_local = policy.crates.get(package);
            if let Some(relative) = value.get("path").and_then(toml::Value::as_str) {
                // Cargo resolves inherited workspace paths from the workspace
                // root, not from the member manifest that opts into them.
                let dependency_root = if inherited { &root } else { &crate_dir };
                let actual = dependency_root
                    .join(relative)
                    .canonicalize()
                    .map_err(|error| {
                        format!("{manifest_path}: invalid dependency path {relative}: {error}")
                    })?;
                let permitted = approved_local
                    .map(|p| root.join(&p.path))
                    .and_then(|p| p.canonicalize().ok());
                if !entry.local_dependencies.contains(package)
                    || permitted.as_ref() != Some(&actual)
                {
                    errors.push(format!(
                        "SG-ARCH-001: forbidden dependency {name} -> {package} ({relative})"
                    ));
                }
            } else if approved_local.is_some() {
                errors.push(format!("SG-ARCH-001: local crate {package} cannot be replaced by a registry/git dependency"));
            } else if !entry.external_dependencies.contains(package) {
                errors.push(format!(
                    "SG-ARCH-001: unreviewed external dependency {name} -> {package}"
                ));
            }
        }
        if name == "slopgate-core"
            && (crate_dir.join("build.rs").exists()
                || manifest
                    .get("package")
                    .and_then(|p| p.get("build"))
                    .is_some())
        {
            errors.push(
                "SG-ARCH-001: core build scripts are not an approved code-generation boundary"
                    .into(),
            );
        }
        let source_dir = crate_dir.join("src");
        for item in walkdir::WalkDir::new(&source_dir).follow_links(false) {
            let item = item.map_err(|error| error.to_string())?;
            if item.file_type().is_symlink() {
                errors.push(format!(
                    "SG-ARCH-001: source symlink is not an approved module boundary: {}",
                    item.path().display()
                ));
                continue;
            }
            if item.path().extension().and_then(|value| value.to_str()) != Some("rs") {
                continue;
            }
            let relative = item
                .path()
                .strip_prefix(&root)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            let text = fs::read_to_string(item.path()).map_err(|error| error.to_string())?;
            let syntax = match syn::parse_file(&text) {
                Ok(syntax) => syntax,
                Err(error) => {
                    errors.push(format!("{relative}: invalid Rust syntax: {error}"));
                    continue;
                }
            };
            let mut visitor = SourceGuard {
                name,
                relative: &relative,
                policy: &policy,
                errors: &mut errors,
            };
            visitor.visit_file(&syntax);
        }
    }
    validate_traceability(&root, &policy, &mut errors)?;
    errors.sort();
    errors.dedup();
    Ok(errors)
}

fn read_owned(root: &Path, relative: &str) -> Result<String, String> {
    if Path::new(relative).is_absolute()
        || Path::new(relative)
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(format!(
            "policy path must be repository-relative: {relative}"
        ));
    }
    let path = root
        .join(relative)
        .canonicalize()
        .map_err(|error| format!("{relative}: {error}"))?;
    if !path.starts_with(root) {
        return Err(format!("policy path escapes repository: {relative}"));
    }
    fs::read_to_string(path).map_err(|error| format!("{relative}: {error}"))
}

fn collect_dependencies(table: &toml::Value, output: &mut Vec<(String, toml::Value)>) {
    for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(dependencies) = table.get(key).and_then(toml::Value::as_table) {
            output.extend(
                dependencies
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }
    }
    if let Some(targets) = table.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            collect_dependencies(target, output);
        }
    }
}

fn is_test(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("test")
            || (attribute.path().is_ident("cfg")
                && attribute
                    .parse_args::<syn::Ident>()
                    .is_ok_and(|value| value == "test"))
    })
}

struct SourceGuard<'a> {
    name: &'a str,
    relative: &'a str,
    policy: &'a Policy,
    errors: &'a mut Vec<String>,
}
impl SourceGuard<'_> {
    fn error(&mut self, message: &str) {
        self.errors.push(format!("{}: {message}", self.relative));
    }
    fn inspect_tokens(&mut self, tokens: &str) {
        if self.relative != self.policy.process_owner
            && matches!(
                tokens,
                "std" | "std :: self" | "std :: *" | "std :: include" | "core :: include"
            )
        {
            self.error("SG-PROC-001: whole-standard-library imports and source-inclusion aliases can bypass resource ownership; import explicit permitted modules instead");
        }
        if self.name == "slopgate-core"
            && (tokens.contains("slopgate_adapters") || tokens.contains("slopgate_rs"))
        {
            self.error("SG-ARCH-001: the neutral core cannot reference an adapter or composition-root crate");
        }
        if self.relative != self.policy.process_owner
            && (tokens.contains("process_wrap")
                || tokens.contains("tokio :: process")
                || (tokens.contains("std :: process")
                    && !(self.name != "slopgate-core"
                        && (tokens.ends_with("process :: exit")
                            || tokens.ends_with("process :: ExitCode")))))
        {
            self.error(
                "SG-PROC-001: production subprocess handling must use the core bounded runner",
            );
        }
    }
}
impl<'ast> Visit<'ast> for SourceGuard<'_> {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        let attributes: &[syn::Attribute] = match item {
            syn::Item::Const(x) => &x.attrs,
            syn::Item::Enum(x) => &x.attrs,
            syn::Item::ExternCrate(x) => &x.attrs,
            syn::Item::Fn(x) => &x.attrs,
            syn::Item::ForeignMod(x) => &x.attrs,
            syn::Item::Impl(x) => &x.attrs,
            syn::Item::Macro(x) => &x.attrs,
            syn::Item::Mod(x) => &x.attrs,
            syn::Item::Static(x) => &x.attrs,
            syn::Item::Struct(x) => &x.attrs,
            syn::Item::Trait(x) => &x.attrs,
            syn::Item::TraitAlias(x) => &x.attrs,
            syn::Item::Type(x) => &x.attrs,
            syn::Item::Union(x) => &x.attrs,
            syn::Item::Use(x) => &x.attrs,
            _ => &[],
        };
        if is_test(attributes) {
            return;
        }
        visit::visit_item(self, item);
    }
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        for path in use_paths(&item.tree, String::new()) {
            self.inspect_tokens(&path);
        }
        visit::visit_item_use(self, item);
    }
    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.inspect_tokens(&path.to_token_stream().to_string());
        visit::visit_path(self, path);
    }
    fn visit_attribute(&mut self, attribute: &'ast syn::Attribute) {
        if attribute.path().is_ident("path") || conditional_path_override(attribute) {
            self.error(
                "SG-ARCH-001: #[path] module redirection is not an approved dependency boundary",
            );
        }
        // Documentation examples are not runtime language dispatch.
        if !attribute.path().is_ident("doc") {
            visit::visit_attribute(self, attribute);
        }
    }
    fn visit_macro(&mut self, invocation: &'ast syn::Macro) {
        if invocation
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "include")
        {
            self.error("SG-ARCH-001: source include! bypasses the approved module graph");
        }
        self.inspect_tokens(&invocation.tokens.to_string());
        if self.name == "slopgate-core" {
            // syn treats macro bodies as token trees. Parse only string literals
            // here, not macro expansion or candidate code execution.
            for literal in string_tokens(
                invocation.tokens.clone(),
                invocation
                    .path
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "json"),
            ) {
                if self.policy.forbidden_core_literals.contains(&literal) {
                    self.error(&format!("SG-ARCH-002: language/checker-specific runtime literal {literal:?} belongs in an adapter"));
                }
            }
        }
        visit::visit_macro(self, invocation);
    }
    fn visit_lit_str(&mut self, literal: &'ast syn::LitStr) {
        if self.name == "slopgate-core"
            && self
                .policy
                .forbidden_core_literals
                .contains(&literal.value())
        {
            self.error(&format!(
                "SG-ARCH-002: language/checker-specific runtime literal {:?} belongs in an adapter",
                literal.value()
            ));
        }
    }
    fn visit_signature(&mut self, signature: &'ast syn::Signature) {
        if signature.unsafety.is_some() && self.relative != self.policy.process_owner {
            self.error("SG-ARCH-001: unsafe functions require a reviewed ownership boundary");
        }
        visit::visit_signature(self, signature);
    }
    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        self.inspect_tokens(&item.ident.to_string());
        if item.ident == "std" && item.rename.is_some() {
            self.error(
                "SG-PROC-001: aliasing the standard crate bypasses explicit resource ownership",
            );
        }
        visit::visit_item_extern_crate(self, item);
    }
    fn visit_expr_unsafe(&mut self, expression: &'ast syn::ExprUnsafe) {
        if self.relative != self.policy.process_owner {
            self.error(
                "SG-ARCH-001: unsafe code requires a separately reviewed ownership boundary",
            );
        }
        visit::visit_expr_unsafe(self, expression);
    }
}

fn conditional_path_override(attribute: &syn::Attribute) -> bool {
    if !attribute.path().is_ident("cfg_attr") {
        return false;
    }
    fn contains_path(meta: &syn::Meta) -> bool {
        if meta.path().is_ident("path") {
            return true;
        }
        match meta {
            syn::Meta::List(list) if list.path.is_ident("cfg_attr") => list
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
                )
                .map_or(true, |items| items.iter().any(contains_path)),
            _ => false,
        }
    }
    match &attribute.meta {
        syn::Meta::List(list) => list
            .parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            )
            .map_or(true, |items| items.iter().any(contains_path)),
        _ => true,
    }
}

fn use_paths(tree: &syn::UseTree, prefix: String) -> Vec<String> {
    match tree {
        syn::UseTree::Path(path) => use_paths(&path.tree, format!("{prefix}{} :: ", path.ident)),
        syn::UseTree::Name(name) => vec![format!("{prefix}{}", name.ident)],
        syn::UseTree::Rename(rename) => vec![format!("{prefix}{}", rename.ident)],
        syn::UseTree::Group(group) => group
            .items
            .iter()
            .flat_map(|tree| use_paths(tree, prefix.clone()))
            .collect(),
        syn::UseTree::Glob(_) => vec![format!("{prefix}*")],
    }
}

fn string_tokens(stream: proc_macro2::TokenStream, json_keys: bool) -> Vec<String> {
    let mut values = Vec::new();
    let mut tokens = stream.into_iter().peekable();
    while let Some(token) = tokens.next() {
        match token {
            proc_macro2::TokenTree::Group(group) => {
                values.extend(string_tokens(group.stream(), json_keys))
            }
            proc_macro2::TokenTree::Literal(literal) => {
                // Serialized field names are schema, not language dispatch. The
                // values still undergo the exact same runtime-literal checks.
                if json_keys
                    && matches!(tokens.peek(), Some(proc_macro2::TokenTree::Punct(p)) if p.as_char() == ':')
                {
                    continue;
                }
                if let Ok(value) = syn::parse_str::<syn::LitStr>(&literal.to_string()) {
                    values.push(value.value());
                }
            }
            _ => {}
        }
    }
    values
}

fn validate_traceability(
    root: &Path,
    policy: &Policy,
    errors: &mut Vec<String>,
) -> Result<(), String> {
    let specification = read_owned(root, &policy.specification)?;
    let expected: BTreeSet<_> = specification
        .split_whitespace()
        .filter_map(|word| word.strip_suffix(':'))
        .filter(|word| word.starts_with("SG-"))
        .map(str::to_string)
        .collect();
    let evidence: EvidenceMap = serde_json::from_str(&read_owned(root, &policy.traceability)?)
        .map_err(|error| format!("traceability map: {error}"))?;
    if evidence.version != 1
        || expected.is_empty()
        || expected != evidence.requirements.keys().cloned().collect()
    {
        errors.push("SG-GOV-001: specification requirements and evidence map differ".into());
    }
    for (id, evidence) in evidence.requirements {
        if evidence.implementation.is_empty() || evidence.tests.is_empty() {
            errors.push(format!(
                "{id}: implementation and executable acceptance evidence are required"
            ));
        }
        for reference in evidence.implementation.iter().chain(evidence.tests.iter()) {
            let (file, symbol) = reference
                .split_once('#')
                .map_or((reference.as_str(), None), |(file, symbol)| {
                    (file, Some(symbol))
                });
            let contents = read_owned(root, file)?;
            if let Some(symbol) = symbol {
                if symbol.is_empty() || !contents.contains(symbol) {
                    errors.push(format!("{id}: missing evidence symbol {reference}"));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn syntax_errors(source: &str, file: &str) -> Vec<String> {
        let policy = Policy {
            version: 1,
            crates: BTreeMap::new(),
            forbidden_core_literals: [".ts".into(), "tsc".into()].into_iter().collect(),
            process_owner: "crates/slopgate-core/src/process.rs".into(),
            specification: String::new(),
            traceability: String::new(),
        };
        let mut errors = Vec::new();
        SourceGuard {
            name: "slopgate-core",
            relative: file,
            policy: &policy,
            errors: &mut errors,
        }
        .visit_file(&syn::parse_file(source).unwrap());
        errors
    }
    #[test]
    fn mutation_core_cannot_import_adapters_or_spawn_processes() {
        for source in [
            "use slopgate_adapters::checkers;",
            "use std::process as process;",
            "fn bad() { std::process::Command::new(\"tool\"); }",
            "use process_wrap::std::CommandWrap;",
            "use std::{process::Command as C};",
            "use std::{process as p};",
            "use std as runtime; fn bad(){runtime::process::Command::new(\"tool\");}",
            "use std::{self as runtime}; fn bad(){runtime::process::Command::new(\"tool\");}",
            "use std::*; fn bad(){process::Command::new(\"tool\");}",
        ] {
            assert!(
                !syntax_errors(source, "crates/slopgate-core/src/gate.rs").is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn mutation_source_redirection_unsafe_and_language_branches_are_rejected() {
        for source in [
            "#[path=\"../../adapters/x.rs\"] mod bad;",
            "include!(\"foreign.rs\");",
            "std::include!(\"foreign.rs\");",
            "use std::include as import_source; import_source!(\"foreign.rs\");",
            "#[cfg_attr(feature=\"foreign\", path=\"../../adapters/x.rs\")] mod bad;",
            "fn bad(p:&str) { if p.ends_with(\".ts\") {} }",
            "fn bad(p:&str) { matches!(p, \"tsc\"); }",
            "fn bad() { unsafe { any(); } }",
            "unsafe fn bad() { any(); }",
        ] {
            assert!(
                !syntax_errors(source, "crates/slopgate-core/src/gate.rs").is_empty(),
                "{source}"
            );
        }
    }
    #[test]
    fn neutral_code_and_test_fixtures_are_allowed() {
        assert!(syntax_errors("fn generic(p: &str, extensions: &[&str]) -> bool { extensions.iter().any(|ext| p.ends_with(ext)) } #[cfg(test)] mod tests { use std::process::Command; fn fixture(){ let _ = \".ts\"; } }", "crates/slopgate-core/src/gate.rs").is_empty());
        assert!(syntax_errors(
            "use std::process::Command; fn spawn(){ let _ = Command::new(\"tool\"); }",
            "crates/slopgate-core/src/process.rs"
        )
        .is_empty());
    }
    #[test]
    fn target_specific_and_dev_dependencies_are_not_hidden() {
        let manifest: toml::Value = toml::from_str("[dev-dependencies]\nslopgate-adapters={path='../slopgate-adapters'}\n[target.'cfg(windows)'.dependencies]\nslopgate-rs={path='../slopgate-rs'}\n").unwrap();
        let mut deps = Vec::new();
        collect_dependencies(&manifest, &mut deps);
        assert_eq!(deps.len(), 2);
    }
    #[test]
    fn telemetry_schema_keys_are_not_language_dispatch() {
        assert!(syntax_errors(
            r#"fn emit() { json!({"ts": timestamp}); }"#,
            "crates/slopgate-core/src/stats/record.rs"
        )
        .is_empty());
        assert!(!syntax_errors(
            r#"fn emit() { json!({"language": "tsc"}); }"#,
            "crates/slopgate-core/src/stats/record.rs"
        )
        .is_empty());
    }
}
