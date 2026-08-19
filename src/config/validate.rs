use std::{path::Path, str::FromStr};

use anyhow::Error;
use regex::Regex;

use super::RawConfig;
use crate::{logger::error, provider::is_valid_provider};

/// Validate the configuration file loaded at the provided path.
///
/// An explicit error is returned if configuration loading fails; otherwise, errors
/// are simply printed to STDERR using the [`logger::error`](crate::logger::error)
/// macro.
pub fn report_config_errors<P: AsRef<Path>>(path: P) -> Result<(), Error> {
    let raw_config = RawConfig::load(path, true)?;

    validate_find_default_template(&raw_config.find.default_template);
    validate_alias_transform_rules(raw_config.alias_transform.rules);
    validate_preferred_keys(&raw_config.preferred_keys);

    Ok(())
}

fn validate_find_default_template(s: &str) {
    if let Err(e) = crate::format::Template::from_str(s) {
        error!("Config 'find.default_template' has invalid syntax: {e}");
    }
}

pub(super) fn check_alias_transform_captures(regex: &Regex) -> Result<(), &'static str> {
    match regex.static_captures_len() {
        Some(2) => Ok(()),
        Some(1) => Err("regex does not contain any capture groups"),
        Some(_) => Err("regex contains too many capture groups"),
        None => Err("some alternatives are either missing or have too many capture groups"),
    }
}

/// Validate alias transform rules for correctness; namely regexes compile, providers are valid,
/// and the regex rules satisfy the 'every alternative contains exactly one capture group' rule
fn validate_alias_transform_rules<S: AsRef<str>, T: AsRef<str>>(
    rules: impl IntoIterator<Item = (S, T)>,
) {
    for (re, provider) in rules {
        let provider = provider.as_ref();
        let re = re.as_ref();
        if !is_valid_provider(provider) {
            error!(
                "Config 'alias_transform.rules' rule [\"{re}\", \"{provider}\"]: contains invalid provider"
            );
        }
        match Regex::new(re) {
            Ok(regex) => {
                if let Err(err) = check_alias_transform_captures(&regex) {
                    error!("Config 'alias_transform.rules' rule [\"{re}\", \"{provider}\"]: {err}");
                }
            }
            Err(e) => {
                error!("Config 'alias_transform.rules' rule [\"{re}\", \"{provider}\"]: {e}");
            }
        }
    }
}

fn validate_preferred_keys(pats: &[String]) {
    for pat in pats {
        if let Err(err) = Regex::new(pat) {
            error!("Config 'preferred_keys' regex \"{pat}\": {err}");
        }
    }
}
