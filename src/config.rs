mod validate;

use std::{cmp::Reverse, fs::read_to_string, io, path::Path, sync::OnceLock};

use anyhow::{Error, anyhow};
use regex::Regex;
use serde::Deserialize;
use toml::from_str;

use crate::{
    Alias, AsKey,
    format::DEFAULT_FIND_TEMPLATE,
    logger::{debug, info, suggest, warn},
    normalize::Normalization,
};
pub use validate::report_config_errors as validate;

/// A direct representation of the default configuration used by library, for easy deserialization
/// from configuration files.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    pub find: RawFindConfig,
    #[serde(default)]
    pub preferred_providers: Vec<String>,
    #[serde(default)]
    pub preferred_keys: Vec<String>,
    #[serde(default)]
    pub alias_transform: RawAutoAlias,
    #[serde(default)]
    pub on_insert: Normalization,
}

fn find_default_template() -> String {
    DEFAULT_FIND_TEMPLATE.into()
}

/// A direct representation of the `[find]` section of the configuration.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RawFindConfig {
    #[serde(default)]
    pub ignore_hidden: bool,
    #[serde(default = "find_default_template")]
    pub default_template: String,
}

impl Default for RawFindConfig {
    fn default() -> Self {
        Self {
            ignore_hidden: Default::default(),
            default_template: find_default_template(),
        }
    }
}

/// A direct representation of the `[auto_alias]` section of the configuration.
#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct RawAutoAlias {
    #[serde(default)]
    rules: Vec<(String, String)>,
    #[serde(default)]
    create_alias: bool,
}

impl RawConfig {
    /// Load configuration by deserializing a toml file at the provided path, returning the default
    /// of `missing_ok` is true.
    fn load<P: AsRef<Path>>(path: P, missing_ok: bool) -> Result<Self, Error> {
        match read_to_string(&path) {
            Ok(st) => {
                info!(
                    "Loading configuration at path '{}'",
                    path.as_ref().display()
                );
                let config = from_str(&st)?;
                debug!("Using configuration:\n{config:?}");
                Ok(config)
            }
            Err(err) => {
                if missing_ok && err.kind() == io::ErrorKind::NotFound {
                    info!(
                        "Configuration file not found at path '{}'; using default configuration",
                        path.as_ref().display()
                    );
                    Ok(Self::default())
                } else {
                    Err(anyhow!("Failed to load configuration file: {err}"))
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct PreferredKeys {
    raw_keys: Vec<String>,
    compiled: OnceLock<Vec<Regex>>,
}

impl PreferredKeys {
    fn compiled_patterns(&self) -> &[Regex] {
        self.compiled.get_or_init(|| {
            self.raw_keys
                .iter()
             .filter_map(|re| {
                Regex::new(re)
                    .inspect_err(|err| warn!("Invalid config: failed to compile 'preferred_keys' regex '{re}': {err}"))
                    .ok()
            })
            .collect()
        })
    }

    fn is_empty(&self) -> bool {
        self.raw_keys.is_empty()
    }

    fn len(&self) -> usize {
        self.raw_keys.len()
    }
}

#[derive(Debug)]
pub struct Config {
    pub find: RawFindConfig,
    pub preferred_keys: PreferredKeys,
    pub alias_transform: LazyAliasTransform,
    pub on_insert: Normalization,
}

impl Config {
    pub fn has_preferred_keys(&self) -> bool {
        !self.preferred_keys.is_empty()
    }

    /// Obtain a score for a key, in terms of preferences defined by the value of
    /// `preferred_keys` in this configuration. If there is no matching prefix,
    /// this returns `None`.
    pub fn preferred_key_matching_idx(&self, id: &str) -> Option<usize> {
        self.preferred_keys
            .compiled_patterns()
            .iter()
            .position(|re| re.is_match(id))
    }

    /// Obtain a score for a key, in terms of preferences defined by the value of
    /// `preferred_keys` in this configuration. Higher scores are better.
    pub fn score_key<'a>(&'a self, id: &str) -> impl Ord + use<'a> {
        Reverse(
            self.preferred_key_matching_idx(id)
                .unwrap_or(self.preferred_keys.len()),
        )
    }
}

#[derive(Debug)]
pub struct LazyAliasTransform {
    compiled: OnceLock<Vec<Regex>>,
    rules: Vec<(String, String)>,
    create_alias: bool,
}

impl LazyAliasTransform {
    fn compiled_rules(&self) -> &[Regex] {
        self.compiled.get_or_init(|| {
            self.rules
                .iter()
             .filter_map(|(re, s)| {
                Regex::new(re)
                    .inspect_err(|err| warn!("Invalid config: failed to compile 'alias_transform.rules' transformation\nrule with provider '{s}': {err}"))
                    .ok()
            })
            .collect()
        })
    }

    fn rule_pairs(&self) -> impl Iterator<Item = (&Regex, &str)> {
        self.compiled_rules()
            .iter()
            .zip(self.rules.iter().map(|(_, provider)| provider.as_ref()))
    }
}

#[cold]
pub fn write_default<W: io::Write>(mut writer: W) -> Result<(), io::Error> {
    writer
        .write(include_str!("config/default_config.toml").as_bytes())
        .map(|_| ())
}

/// Attempt to load the configuration file from the provided path.
///
/// If `missing_ok` is true and the file is not found, this returns the default configuration.
pub fn load<P: AsRef<Path>>(path: P, missing_ok: bool) -> Result<Config, Error> {
    let RawConfig {
        find,
        preferred_providers,
        mut preferred_keys,
        alias_transform: RawAutoAlias {
            rules,
            create_alias,
        },
        on_insert,
    } = RawConfig::load(path, missing_ok)?;

    if !preferred_providers.is_empty() {
        if preferred_keys.is_empty() {
            warn!(
                "Configuration key `preferred_providers` has been deprecated; it is replaced by `preferred_keys`."
            );
            suggest!(
                "In your configuration file, rename `preferred_providers` to `preferred_keys` and replace each `provider` with the regex `^provider:.*`"
            );
            preferred_keys = preferred_providers;
            for k in preferred_keys.iter_mut() {
                k.insert(0, '^');
                k.push_str(":.*");
            }
        } else {
            anyhow::bail!(
                "Configuration defines both `preferred_providers` and `preferred_keys`. `preferred_providers` has been deprecated;"
            )
        }
    }

    let alias_transform = LazyAliasTransform {
        compiled: OnceLock::new(),
        rules,
        create_alias,
    };

    let preferred_keys = PreferredKeys {
        raw_keys: preferred_keys,
        compiled: OnceLock::new(),
    };

    Ok(Config {
        find,
        preferred_keys,
        alias_transform,
        on_insert,
    })
}

pub trait AliasTransform {
    /// Iterate over the internal matching patterns and return a pair (provider, sub_id) if one of
    /// the matches succeeds. The default implementation automatically fails.
    fn map_alias<'a>(&'a self, _alias: &'a Alias) -> Option<(&'a str, &'a str)> {
        None
    }

    /// Whether or not to save the alias in the the `Keys` table after mapping.
    fn create(&self) -> bool {
        false
    }
}

impl AliasTransform for () {}

impl AliasTransform for LazyAliasTransform {
    fn map_alias<'a>(&'a self, alias: &'a Alias) -> Option<(&'a str, &'a str)> {
        for (re, provider) in self.rule_pairs() {
            if let Some((_, [res])) = re.captures(alias.as_key()).map(|caps| caps.extract()) {
                return Some((provider, res));
            }
        }

        None
    }

    fn create(&self) -> bool {
        self.create_alias
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let mut default_config_bytes = Vec::new();
        write_default(&mut default_config_bytes).unwrap();
        let st = String::from_utf8(default_config_bytes).unwrap();
        let cfg: RawConfig = from_str(&st).unwrap();

        assert_eq!(cfg, RawConfig::default());
    }
}
