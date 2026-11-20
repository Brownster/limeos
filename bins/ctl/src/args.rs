//! Shared command-line rules. Exit status 0 means done, 1 means refused or
//! failed (nothing more was done than the message says), 2 means the command
//! line was malformed and nothing was done at all.
//!
//! `--help`, `-h` and `--version` count only in command position: first, or
//! straight after the subcommand words. Anywhere else they are ordinary words,
//! so `--reason --version` is a usage error, never a silent "success".

/// A malformed command line: nothing was done. Exit status 2.
#[derive(Debug)]
pub struct Usage(pub String);

impl std::fmt::Display for Usage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Usage {}

pub fn is_help(arg: &str) -> bool {
    arg == "--help" || arg == "-h"
}

/// Help asked for straight after a subcommand word or its own subcommand:
/// `job --help`, `job resolve --help`, never after an option.
pub fn help_requested(args: &[String]) -> bool {
    match args {
        [first, ..] if is_help(first) => true,
        [first, second, ..] => !first.starts_with('-') && is_help(second),
        _ => false,
    }
}

/// The value that follows `flag`. A missing value, or one that looks like a
/// flag, is a usage error rather than being taken as the value.
pub fn value<'a>(flag: &str, next: Option<&'a String>) -> Result<&'a String, Usage> {
    match next {
        Some(value) if !value.starts_with('-') => Ok(value),
        Some(value) => Err(Usage(format!(
            "{flag} needs a value, not {value}; run limeosctl --help"
        ))),
        None => Err(Usage(format!("missing value for {flag}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_is_never_a_flag() {
        let given = |s: &str| Some(s.to_owned());
        assert_eq!(value("--out", given("x").as_ref()).unwrap(), "x");
        for flagish in ["--version", "--help", "-h", "-", "--"] {
            let error = value("--reason", given(flagish).as_ref()).unwrap_err();
            assert!(error.0.contains("--reason needs a value"), "{error}");
        }
        assert!(value("--out", None).is_err());
    }

    #[test]
    fn help_counts_only_in_command_position() {
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(help_requested(&args(&["--help"])));
        assert!(help_requested(&args(&["resolve", "-h"])));
        assert!(!help_requested(&args(&["--admin", "--help"])));
        assert!(!help_requested(&args(&["resolve", "j", "--help"])));
        assert!(!help_requested(&args(&[])));
    }
}
