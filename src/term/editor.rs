use std::{cmp::PartialEq, fmt::Display, io::Result, str::FromStr};

use edit::{Builder, edit_with_builder};

use super::Confirm;

pub struct EditorConfig {
    /// The suffix for the temporary file.
    pub suffix: &'static str,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self { suffix: ".txt" }
    }
}

pub struct Editor {
    inner: Builder<'static, 'static>,
}

impl Default for Editor {
    fn default() -> Self {
        Self::new(EditorConfig::default())
    }
}

impl Editor {
    /// Initialize a new editor using the [`EditorConfig`].
    pub fn new(config: EditorConfig) -> Self {
        let mut inner = Builder::new();
        inner.suffix(config.suffix);
        Self { inner }
    }

    /// Edit the object and optionally return a new object. This will repeatedly prompt the user to
    /// edit until the object is changed. If this returns `Ok(Some(object)`, the new `object` is
    /// guaranteed to be different than the old object. This returns `Ok(None)` if the user cancelled
    /// the edit.
    pub fn edit<T: ToString + FromStr + PartialEq>(&self, object: &T) -> Result<Option<T>>
    where
        <T as FromStr>::Err: Display,
    {
        let prompter = Confirm::new("Continue editing?", true);
        let mut response = object.to_string();

        loop {
            let user_text = edit_with_builder(&response, &self.inner)?;

            // the text was unchanged
            if user_text == response {
                break Ok(None);
            }

            match T::from_str(&user_text) {
                Ok(new_object) => {
                    if &new_object != object {
                        break Ok(Some(new_object));
                    } else {
                        eprint!("Text edited but contents unchanged! ");
                    }
                }
                Err(err) => {
                    eprint!("Contents invalid: {err}");
                }
            }

            if prompter.confirm()? {
                response = user_text;
            } else {
                break Ok(None);
            }
        }
    }
}
