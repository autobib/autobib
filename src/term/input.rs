use std::{
    fmt::Display,
    io::{Result, Write, stderr, stdin},
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
        {
            let mut stderr = stderr().lock();
            write!(stderr, "{}: ", self.message)?;
            stderr.flush()?;
        }
        // lock is released as it goes out of scope

        let mut input = String::new();
        stdin().read_line(&mut input)?;

        input.truncate(input.trim_end().len());

        Ok(input)
    }
}
