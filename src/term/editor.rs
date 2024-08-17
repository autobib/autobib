use std::cmp::PartialEq;
use std::io::{stdout, Result};
use std::str::FromStr;

use crossterm::{
    event::{read, Event, KeyCode},
    execute, style,
};
use edit::{edit_with_builder, Builder};

pub struct Config {
    /// The suffix for the temporary file.
    pub suffix: &'static str,
}

impl Default for Config {
    fn default() -> Self {
        Self { suffix: ".txt" }
    }
}

pub struct Editor {
    config: Config,
}

impl Editor {
    /// Initialize a new editor using the [`Config`].
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Edit the object and optionally return a new object. This will repeatedly prompt the user to
    /// edit until the object is changed. If this returns successfully, the new object is
    /// guaranteed to be different than the old object. This returns `None` if the user cancelled
    /// the edit.
    pub fn edit<T: ToString + FromStr + PartialEq>(&self, object: &T) -> Result<Option<T>> {
        let mut editor = Builder::new();
        editor.suffix(self.config.suffix);

        let mut response = object.to_string();

        loop {
            let user_text = edit_with_builder(&response, &editor)?;
            match T::from_str(&user_text) {
                Ok(new_object) => {
                    if &new_object != object {
                        break Ok(Some(new_object));
                    } else {
                        eprint!("Contents unchanged! ");
                    }
                }
                Err(_) => {
                    eprint!("Contents invalid! ");
                }
            }

            execute!(stdout(), style::Print("Continue editing? [Y]/n "))?;

            match read()? {
                Event::Key(key)
                    if key.code == KeyCode::Enter
                        || key.code == KeyCode::Char('y')
                        || key.code == KeyCode::Char('Y') =>
                {
                    response = user_text;
                }
                _ => break Ok(None),
            }
        }
    }
}
