//! Bundled skill tree installer. Mirrors `src/install-skills.mjs`.
//!
//! Phase 1 takes `skills_src` explicitly (JS derives it from `import.meta.url`).

use crate::error::SlopError;
use crate::install::agent_hooks::home_dir;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// What happened when installing one skill directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillInstallAction {
    Skipped,
    Installed,
    Updated,
}

/// Per-skill outcome from [`install_skills`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInstallResult {
    pub name: String,
    pub action: SkillInstallAction,
}

/// Default destination under `home` (`~/.claude/skills` when `home` is `$HOME`).
pub fn default_skills_dest_in(home: &Path) -> PathBuf {
    home.join(".claude").join("skills")
}

/// Default destination (`~/.claude/skills`), matching the JS default.
pub fn default_skills_dest() -> PathBuf {
    default_skills_dest_in(&home_dir())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), SlopError> {
    for entry in WalkDir::new(src) {
        let entry = entry.map_err(|e| SlopError::Io(format!("walk {}: {e}", src.display())))?;
        let path = entry.path();
        let relative = path
            .strip_prefix(src)
            .map_err(|e| SlopError::Io(format!("strip_prefix {}: {e}", src.display())))?;
        let target = dst.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)
                .map_err(|e| SlopError::Io(format!("mkdir {}: {e}", target.display())))?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| SlopError::Io(format!("mkdir {}: {e}", parent.display())))?;
            }
            fs::copy(path, &target).map_err(|e| {
                SlopError::Io(format!(
                    "copy {} -> {}: {e}",
                    path.display(),
                    target.display()
                ))
            })?;
        }
    }
    Ok(())
}

/// Copy bundled skill directories from `skills_src` into `dest`.
///
/// Each immediate child directory of `skills_src` is treated as one skill.
/// Existing destinations are skipped unless `force` is true. Missing
/// `skills_src` yields an empty result (JS: early return).
pub fn install_skills(
    skills_src: &Path,
    dest: &Path,
    force: bool,
) -> Result<Vec<SkillInstallResult>, SlopError> {
    if !skills_src.is_dir() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    let entries = fs::read_dir(skills_src)
        .map_err(|e| SlopError::Io(format!("read_dir {}: {e}", skills_src.display())))?;

    for entry in entries {
        let entry = entry
            .map_err(|e| SlopError::Io(format!("read_dir entry {}: {e}", skills_src.display())))?;
        let file_type = entry
            .file_type()
            .map_err(|e| SlopError::Io(format!("file_type {}: {e}", entry.path().display())))?;
        if !file_type.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        let target = dest.join(&name);

        if target.exists() && !force {
            results.push(SkillInstallResult {
                name,
                action: SkillInstallAction::Skipped,
            });
            continue;
        }

        let existed = target.exists();
        fs::create_dir_all(&target)
            .map_err(|e| SlopError::Io(format!("mkdir {}: {e}", target.display())))?;
        copy_dir_recursive(&entry.path(), &target)?;

        let action = if force && existed {
            SkillInstallAction::Updated
        } else {
            SkillInstallAction::Installed
        };
        results.push(SkillInstallResult { name, action });
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_skill_tree(src: &Path, skill: &str, content: &str) {
        let skill_dir = src.join(skill);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn default_skills_dest_under_home_claude_skills() {
        let home = TempDir::new().unwrap();
        let path = default_skills_dest_in(home.path());
        assert_eq!(path, home.path().join(".claude/skills"));
    }

    #[test]
    fn install_skills_copies_source_tree_into_dest() {
        let src_dir = TempDir::new().unwrap();
        let dest_dir = TempDir::new().unwrap();
        write_skill_tree(src_dir.path(), "slopgate-init", "# init skill\n");
        write_skill_tree(src_dir.path(), "slopgate-ux", "# ux skill\n");

        let results = install_skills(src_dir.path(), dest_dir.path(), false).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results
            .iter()
            .all(|r| r.action == SkillInstallAction::Installed));

        let init_md = dest_dir.path().join("slopgate-init/SKILL.md");
        let ux_md = dest_dir.path().join("slopgate-ux/SKILL.md");
        assert!(init_md.is_file());
        assert!(ux_md.is_file());
        assert_eq!(fs::read_to_string(init_md).unwrap(), "# init skill\n");
        assert_eq!(fs::read_to_string(ux_md).unwrap(), "# ux skill\n");
    }

    #[test]
    fn install_skills_second_call_without_force_skips_existing() {
        let src_dir = TempDir::new().unwrap();
        let dest_dir = TempDir::new().unwrap();
        write_skill_tree(src_dir.path(), "slopgate-init", "version-one\n");

        let first = install_skills(src_dir.path(), dest_dir.path(), false).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].action, SkillInstallAction::Installed);

        write_skill_tree(src_dir.path(), "slopgate-init", "version-two\n");
        let second = install_skills(src_dir.path(), dest_dir.path(), false).unwrap();
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].action, SkillInstallAction::Skipped);

        let dest_md = dest_dir.path().join("slopgate-init/SKILL.md");
        assert_eq!(fs::read_to_string(dest_md).unwrap(), "version-one\n");
    }

    #[test]
    fn install_skills_force_overwrites_existing() {
        let src_dir = TempDir::new().unwrap();
        let dest_dir = TempDir::new().unwrap();
        write_skill_tree(src_dir.path(), "slopgate-init", "version-one\n");

        install_skills(src_dir.path(), dest_dir.path(), false).unwrap();
        write_skill_tree(src_dir.path(), "slopgate-init", "version-two\n");

        let forced = install_skills(src_dir.path(), dest_dir.path(), true).unwrap();
        assert_eq!(forced.len(), 1);
        assert_eq!(forced[0].action, SkillInstallAction::Updated);

        let dest_md = dest_dir.path().join("slopgate-init/SKILL.md");
        assert_eq!(fs::read_to_string(dest_md).unwrap(), "version-two\n");
    }

    #[test]
    fn install_skills_unhappy_missing_source_returns_empty() {
        let dest_dir = TempDir::new().unwrap();
        let missing = dest_dir.path().join("no-skills-here");
        let results = install_skills(&missing, dest_dir.path(), false).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn install_skills_unhappy_skips_non_directory_entries() {
        let src_dir = TempDir::new().unwrap();
        let dest_dir = TempDir::new().unwrap();
        write_skill_tree(src_dir.path(), "real-skill", "ok\n");
        fs::write(src_dir.path().join("README.md"), "not a skill dir\n").unwrap();

        let results = install_skills(src_dir.path(), dest_dir.path(), false).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "real-skill");
        assert!(!dest_dir.path().join("README.md").exists());
    }
}
