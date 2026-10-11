// SPDX-License-Identifier: Apache-2.0

//! A small argument parser: `--name value`, `--name=value`, `--flag` and positional arguments.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::str::FromStr;

/// Parsed arguments.
#[derive(Debug, Default)]
pub struct Args {
    positional: Vec<String>,
    options: BTreeMap<String, Vec<String>>,
    flags: Vec<String>,
}

/// A usage error, printed with the usage text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageError(pub String);

impl Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Args {
    /// Parses `args`; `flags` names the options that take no value.
    pub fn parse(
        args: impl IntoIterator<Item = String>,
        flags: &[&str],
    ) -> Result<Self, UsageError> {
        let mut parsed = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if arg == "--" {
                parsed.positional.extend(args.by_ref());
                break;
            }
            let Some(name) = arg.strip_prefix("--") else {
                parsed.positional.push(arg);
                continue;
            };
            if let Some((name, value)) = name.split_once('=') {
                parsed
                    .options
                    .entry(name.to_owned())
                    .or_default()
                    .push(value.to_owned());
            } else if flags.contains(&name) {
                parsed.flags.push(name.to_owned());
            } else {
                let value = args
                    .next()
                    .ok_or_else(|| UsageError(format!("--{name} needs a value")))?;
                parsed
                    .options
                    .entry(name.to_owned())
                    .or_default()
                    .push(value);
            }
        }
        Ok(parsed)
    }

    /// The positional arguments.
    pub fn positional(&self) -> &[String] {
        &self.positional
    }

    /// Whether `--name` was given as a flag.
    pub fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|flag| flag == name)
    }

    /// The last value of `--name`, if given.
    pub fn value(&self, name: &str) -> Option<&str> {
        self.options
            .get(name)
            .and_then(|values| values.last())
            .map(String::as_str)
    }

    /// Every value of `--name`.
    pub fn values(&self, name: &str) -> impl Iterator<Item = &str> {
        self.options
            .get(name)
            .into_iter()
            .flatten()
            .map(String::as_str)
    }

    /// `--name` parsed as `T`, or `default` when not given.
    pub fn parsed<T: FromStr>(&self, name: &str, default: T) -> Result<T, UsageError>
    where
        T::Err: Display,
    {
        match self.value(name) {
            None => Ok(default),
            Some(value) => value
                .parse()
                .map_err(|error| UsageError(format!("--{name} {value}: {error}"))),
        }
    }

    /// `--name` as a channel number, `none` for no channel, or `default` when not given.
    pub fn channel(&self, name: &str, default: Option<u8>) -> Result<Option<u8>, UsageError> {
        match self.value(name) {
            None => Ok(default),
            Some("none") | Some("-") => Ok(None),
            Some(value) => value
                .parse::<u8>()
                .map(Some)
                .map_err(|error| UsageError(format!("--{name} {value}: {error}"))),
        }
    }

    /// Checks that only `known` options and flags were given.
    pub fn only(&self, known: &[&str]) -> Result<(), UsageError> {
        for name in self
            .options
            .keys()
            .map(String::as_str)
            .chain(self.flags.iter().map(String::as_str))
        {
            if !known.contains(&name) {
                return Err(UsageError(format!("unknown option --{name}")));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Args {
        Args::parse(
            args.iter().map(|arg| (*arg).to_owned()),
            &["lsb-first", "stats"],
        )
        .expect("parses")
    }

    #[test]
    fn reads_options_flags_and_positionals() {
        let args = parse(&[
            "decode",
            "--rate",
            "1000",
            "--mode=3",
            "--lsb-first",
            "in.bin",
            "--cs",
            "none",
        ]);
        assert_eq!(args.positional(), &["decode", "in.bin"]);
        assert_eq!(args.value("rate"), Some("1000"));
        assert_eq!(args.parsed("mode", 0u8), Ok(3));
        assert!(args.flag("lsb-first"));
        assert!(!args.flag("stats"));
        assert_eq!(args.channel("cs", Some(1)), Ok(None));
        assert_eq!(args.channel("clk", Some(0)), Ok(Some(0)));
        assert_eq!(args.parsed("rate", 0u64), Ok(1000));
        assert!(args.parsed("mode", 0u8).is_ok());
        assert_eq!(args.only(&["rate", "mode", "lsb-first", "cs"]), Ok(()));
        assert!(args.only(&["rate"]).is_err());
    }

    #[test]
    fn rejects_missing_values_and_bad_numbers() {
        let error = Args::parse(["--rate".to_owned()], &[]).expect_err("fails");
        assert_eq!(error.0, "--rate needs a value");
        let args = parse(&["--mode", "x"]);
        assert!(args.parsed("mode", 0u8).is_err());
    }
}
