use std::io::{stdin, stdout, Result, Write};

pub struct Confirm {
    /// The message to display before the prompt text.
    message: &'static str,
    /// The default value for confirmation.
    default: bool,
}

impl Confirm {
    pub fn new(message: &'static str, default: bool) -> Self {
        Self { message, default }
    }

    pub fn confirm(&self) -> Result<bool> {
        let mut stdout = stdout();
        write!(stdout, "{}", self.message)?;
        write!(stdout, " ")?;
        if self.default {
            write!(stdout, "[Y]/n")?;
        } else {
            write!(stdout, "y/[N]")?;
        }
        write!(stdout, " ")?;
        stdout.flush()?;

        let mut input = String::new();
        stdin().read_line(&mut input)?;

        Ok(match input.trim() {
            "y" | "Y" => true,
            "n" | "N" => false,
            "" => self.default,
            _ => false,
        })
    }
}
