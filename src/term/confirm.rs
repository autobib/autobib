use std::{fmt::Display, io::Result};

use super::Input;

struct ConfirmPrompt<S> {
    /// The message to display before the prompt text.
    message: S,
    /// The default value for confirmation.
    default: bool,
}

impl<S: Display> Display for ConfirmPrompt<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)?;
        f.write_str(" ")?;
        if self.default {
            f.write_str("[Y]/n")
        } else {
            f.write_str("y/[N]")
        }
    }
}

pub struct Confirm<S> {
    inner: Input<ConfirmPrompt<S>>,
    default: bool,
}

impl<S: Display> Confirm<S> {
    pub fn new(message: S, default: bool) -> Self {
        Self {
            inner: Input::new(ConfirmPrompt { message, default }),
            default,
        }
    }

    pub fn confirm(&self) -> Result<bool> {
        Ok(match self.inner.input()?.trim() {
            "y" | "Y" => true,
            "n" | "N" => false,
            "" => self.default,
            _ => false,
        })
    }
}
