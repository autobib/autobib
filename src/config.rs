mod validate;

use std::{fs::read_to_string, io, path::Path, sync::LazyLock};

use anyhow::{Error, anyhow};
use regex::Regex;
use serde::Deserialize;
use toml::from_str;

use crate::{
    Alias, CitationKey,
    format::DEFAULT_FIND_TEMPLATE,
    logger::{debug, info, warn},
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
pub struct Config<F> {
    pub find: RawFindConfig,
    pub preferred_providers: Vec<String>,
    pub alias_transform: LazyAliasTransform<F>,
    pub on_insert: Normalization,
}

#[derive(Debug)]
pub struct LazyAliasTransform<F> {
    rules: LazyLock<Vec<(Regex, String)>, F>,
    create_alias: bool,
}

#[cold]
pub fn write_default<W: ?Sized + io::Write>(writer: &mut W) -> Result<(), io::Error> {
    writer
        .write(include_str!("config/default_config.toml").as_bytes())
        .map(|_| ())
}

/// Attempt to load the configuration file from the provided path.
///
/// If `missing_ok` is true and the file is not found, this returns the default configuration.
pub fn load<P: AsRef<Path>>(
    path: P,
    missing_ok: bool,
) -> Result<Config<impl FnOnce() -> Vec<(Regex, String)>>, Error> {
    let RawConfig {
        find,
        preferred_providers,
        alias_transform: RawAutoAlias {
            rules,
            create_alias,
        },
        on_insert,
    } = RawConfig::load(path, missing_ok)?;

    let rules = LazyLock::new(move || {
        rules
            .into_iter()
            .filter_map(|(re, s)| {
                Regex::new(&re)
                    .inspect_err(|err| warn!("Invalid config: failed to compile 'alias_transform.rules' transformation\nrule with provider '{s}': {err}"))
                    .ok()
                    .map(|compiled| (compiled, s))
            })
            .collect()
    });

    let alias_transform = LazyAliasTransform {
        rules,
        create_alias,
    };

    Ok(Config {
        find,
        preferred_providers,
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

    /// Whether or not to save the alias in the the `CitationKeys` table after mapping.
    fn create(&self) -> bool {
        false
    }
}

impl AliasTransform for () {}

impl<F: FnOnce() -> Vec<(Regex, String)>> AliasTransform for LazyAliasTransform<F> {
    fn map_alias<'a>(&'a self, alias: &'a Alias) -> Option<(&'a str, &'a str)> {
        for (re, provider) in self.rules.iter() {
            if let Some(cap) = re.captures(alias.name())
                && let Some(res) = cap.get(1)
            {
                return Some((provider, res.as_str()));
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
