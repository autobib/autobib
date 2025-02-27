use std::{
    fmt::Display,
    io::{Result, Write, stdin, stdout},
};

pub struct Input<S> {
    /// The message to display before the prompt text.
    message: S,
}

impl<S: Display> Input<S> {
    pub fn new(message: S) -> Self {
        Self { message }
    }

    pub fn input(&self) -> Result<String> {
        let mut stdout = stdout();
        write!(stdout, "{}: ", self.message)?;
        stdout.flush()?;

        let mut input = String::new();
        stdin().read_line(&mut input)?;

        input.truncate(input.trim_end().len());

        Ok(input)
    }
}
