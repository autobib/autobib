use std::{
    cmp::PartialEq,
    io::{stdin, stdout, Result, Write},
    str::FromStr,
};

use edit::{edit_with_builder, Builder};

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
    config: EditorConfig,
}

impl Editor {
    /// Initialize a new editor using the [`EditorConfig`].
    pub fn new(config: EditorConfig) -> Self {
        Self { config }
    }

    /// Edit the object and optionally return a new object. This will repeatedly prompt the user to
    /// edit until the object is changed. If this returns `Ok(Some(object)`, the new `object` is
    /// guaranteed to be different than the old object. This returns `Ok(None)` if the user cancelled
    /// the edit.
    pub fn edit<T: ToString + FromStr + PartialEq>(&self, object: &T) -> Result<Option<T>> {
        let mut editor = Builder::new();
        editor.suffix(self.config.suffix);

        let mut response = object.to_string();

        loop {
            let user_text = edit_with_builder(&response, &editor)?;

            // the text was unchanged
            if user_text == response {
                eprintln!("Aborted!");
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
                Err(_) => {
                    eprint!("Contents invalid! ");
                }
            }

            eprint!("Continue editing? [Y]/n ");
            stdout().flush()?;

            let mut input = String::new();
            stdin().read_line(&mut input)?;

            match input.trim() {
                "" | "y" | "Y" => response = user_text,
                _ => break Ok(None),
            }
        }
    }
}
