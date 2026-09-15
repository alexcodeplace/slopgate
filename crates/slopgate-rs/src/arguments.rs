//! Position-aware parsing for the legacy CLI surface. Option values are data,
//! never commands: `--file init` must not initialize a repository.
use std::collections::HashSet;
const VALUES: &[&str] = &[
    "--agent",
    "--by",
    "--class",
    "--config",
    "--file",
    "--fingerprint",
    "--format",
    "--line",
    "--scope",
    "--since",
    "--since-days",
    "--source",
    "--tier",
];
const FLAGS: &[&str] = &[
    "--check",
    "--force",
    "--help",
    "-h",
    "--json",
    "--prune",
    "--self-test",
    "--staged",
    "--update",
    "--version",
];
const COMMANDS: &[&str] = &[
    "agent-hooks",
    "audit",
    "baseline",
    "capabilities",
    "defect",
    "doctor",
    "harvest",
    "help",
    "init",
    "install-hooks",
    "install-skills",
    "scan",
    "stats",
];

fn positions(args: &[String]) -> Vec<usize> {
    let mut index = 1;
    let mut output = Vec::new();
    while index < args.len() {
        output.push(index);
        index += if VALUES.contains(&args[index].as_str()) {
            2
        } else {
            1
        };
    }
    output
}

pub fn has(args: &[String], flag: &str) -> bool {
    if !flag.starts_with('-') {
        return positions(args)
            .into_iter()
            .find(|index| !args[*index].starts_with('-'))
            .is_some_and(|index| args[index] == flag);
    }
    positions(args).iter().any(|index| args[*index] == flag)
}

pub fn val_of<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let index = positions(args)
        .into_iter()
        .find(|index| args[*index] == flag)?;
    let value = args.get(index + 1)?;
    if !flag.starts_with('-') && value.starts_with('-') {
        return None;
    }
    Some(value)
}

pub fn validate(args: &[String]) -> Result<(), String> {
    let mut options = HashSet::new();
    let mut positional = Vec::new();
    for index in positions(args) {
        let value = args[index].as_str();
        if value.starts_with('-') {
            if !VALUES.contains(&value) && !FLAGS.contains(&value) {
                return Err(format!("slopgate: unknown option {value:?}"));
            }
            if !options.insert(value) {
                return Err(format!("slopgate: duplicate option {value}"));
            }
            if VALUES.contains(&value) && args.get(index + 1).is_none_or(|value| value.is_empty()) {
                return Err(format!("slopgate: {value} requires a value"));
            }
        } else {
            positional.push(value);
        }
    }
    let command = positional.first().copied();
    if command.is_some_and(|command| !COMMANDS.contains(&command)) {
        return Err(format!(
            "slopgate: unknown command {:?}",
            command.unwrap_or_default()
        ));
    }
    let max_positionals = if matches!(command, Some("init" | "agent-hooks" | "defect")) {
        2
    } else {
        1
    };
    if positional.len() > max_positionals {
        return Err("slopgate: unexpected positional argument".into());
    }
    let file_gate = options.contains("--file") && command != Some("defect");
    let modes = usize::from(file_gate)
        + usize::from(options.contains("--staged"))
        + usize::from(options.contains("--self-test"))
        + usize::from(command == Some("scan"));
    if modes > 1 {
        return Err("slopgate: choose exactly one scan mode".into());
    }
    if modes > 0 && command.is_some_and(|command| command != "scan") {
        return Err("slopgate: scan flags cannot accompany a different command".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        std::iter::once("slopgate")
            .chain(values.iter().copied())
            .map(str::to_string)
            .collect()
    }
    #[test]
    fn command_names_and_flags_inside_option_values_are_data() {
        for name in ["init", "baseline", "scan", "--help", "--staged"] {
            let input = args(&["--file", name, "--config", "config.toml"]);
            assert!(validate(&input).is_ok());
            assert_eq!(val_of(&input, "--file"), Some(name));
            assert!(!has(&input, "init"));
            assert!(!has(&input, "baseline"));
            assert!(!has(&input, "scan"));
            assert!(!has(&input, "--help"));
            assert!(!has(&input, "--staged"));
        }
    }
    #[test]
    fn unknown_duplicate_missing_and_conflicting_options_fail() {
        for input in [
            vec!["--mystery"],
            vec!["--config"],
            vec!["--staged", "--staged"],
            vec!["--file", "a", "--staged"],
            vec!["scan", "--file", "a"],
        ] {
            assert!(validate(&args(&input)).is_err(), "{input:?}");
        }
    }
}
