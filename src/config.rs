use std::{fs::read_to_string, io::ErrorKind, path::Path};

use anyhow::{anyhow, Error};
use serde::{Deserialize, Serialize};
use toml::from_str;

use crate::{
    logger::{debug, info},
    normalize::Normalization,
};

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub on_insert: Normalization,
}

impl Config {
    /// Attempt to load the configuration file from the provided path.
    ///
    /// If `missing_ok` is true and the file is not found, this returns the default configuration.
    pub fn load<P: AsRef<Path>>(path: P, missing_ok: bool) -> Result<Self, Error> {
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
                if missing_ok && err.kind() == ErrorKind::NotFound {
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
