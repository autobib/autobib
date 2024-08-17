//! A generic picker implementation based on the [`nucleo`] fuzzy matcher.
use std::cmp::min;
use std::io::{self, Stdout, Write};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam::channel::{unbounded, Receiver};
use crossterm::{
    cursor,
    event::{read, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode},
    execute, style,
    terminal::{
        self, disable_raw_mode, enable_raw_mode, size, EnterAlternateScreen, LeaveAlternateScreen,
    },
    QueueableCommand,
};
use nucleo::{Config, Injector, Nucleo, Utf32String};

struct TerminalState {
    /// The width of the screen.
    width: u16,
    /// The height of the screen, including the prompt.
    height: u16,
    /// The selector index position, or [`None`] if there is nothing to select.
    selector_index: Option<u16>,
    /// The position of the cursor within the query.
    query_index: u16,
    /// The query string.
    query: String,
    /// The current number of items to be drawn to the terminal.
    draw_count: u16,
}

impl TerminalState {
    /// Initialize a new terminal state.
    pub fn new(dimensions: (u16, u16)) -> Self {
        let (width, height) = dimensions;
        Self {
            width,
            height,
            selector_index: None,
            query_index: 0,
            query: String::new(),
            draw_count: 0,
        }
    }

    /// Increment the current item selection.
    pub fn incr_selection(&mut self) {
        self.selector_index = self.selector_index.map(|i| i + 1);
        self.clamp_selector_index();
    }

    /// Decrement the current item selection.
    pub fn decr_selection(&mut self) {
        self.selector_index = self.selector_index.map(|i| if i > 0 { i - 1 } else { 0 });
        self.clamp_selector_index();
    }

    /// Update the draw count from a snapshot.
    pub fn update_draw_count<T: Send + Sync + 'static>(&mut self, snapshot: &nucleo::Snapshot<T>) {
        self.draw_count = snapshot.matched_item_count().try_into().unwrap_or(u16::MAX);
        self.clamp_draw_count();
        self.clamp_selector_index();
    }

    /// Clamp the draw count so that it falls in the valid range.
    fn clamp_draw_count(&mut self) {
        self.draw_count = min(self.draw_count, self.height - 1)
    }

    /// Clamp the selector index so that it falls in the valid range.
    fn clamp_selector_index(&mut self) {
        if self.draw_count == 0 {
            self.selector_index = None;
        } else {
            let position = min(self.selector_index.unwrap_or(0), self.draw_count - 1);
            self.selector_index = Some(position);
        }
    }

    /// Append a char to the query string.
    pub fn push_char(&mut self, ch: char) {
        self.query.push(ch);
        self.query_index += 1;
    }

    /// Delete a char from the query string.
    pub fn del_char(&mut self) {
        if self.query.pop().is_some() {
            self.query_index -= 1;
        }
    }

    /// Format a [`Utf32String`] for displaying. Currently:
    /// - Delete control characters.
    /// - Truncates the string to an appropriate length.
    /// - Replaces any newline characters with spaces.
    fn format_display(&self, display: &Utf32String) -> String {
        display
            .slice(..)
            .chars()
            .filter(|ch| !ch.is_control())
            .take(self.width as usize - 2)
            .map(|ch| match ch {
                '\n' => ' ',
                s => s,
            })
            .collect()
    }

    /// Draw the terminal to the screen. This assumes that the draw count has been updated and the
    /// selector index has been properly clamped, or this method will panic!
    fn draw<T: Send + Sync + 'static>(
        &mut self,
        stdout: &mut Stdout,
        snapshot: &nucleo::Snapshot<T>,
    ) -> Result<(), io::Error> {
        // clear screen and set cursor position to bottom
        stdout
            .queue(terminal::Clear(terminal::ClearType::All))?
            .queue(cursor::MoveTo(0, self.height - 1))?;

        // draw the matches
        for it in snapshot.matched_items(..self.draw_count as u32) {
            let render = self.format_display(&it.matcher_columns[0]);
            stdout
                .queue(cursor::MoveUp(1))?
                .queue(cursor::MoveToColumn(2))?
                .queue(style::Print(render))?;
        }

        // draw the selection indicator
        if let Some(position) = self.selector_index {
            stdout
                .queue(cursor::MoveTo(0, self.height - 2 - position))?
                .queue(style::Print("*"))?;
        }

        // render the query string
        stdout
            .queue(cursor::MoveTo(0, self.height - 1))?
            .queue(style::Print("> "))?
            .queue(style::Print(&self.query))?;

        // flush to terminal
        stdout.flush()
    }

    /// Resize the terminal state on screen size change.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        self.clamp_draw_count();
        self.clamp_selector_index();
    }
}

/// The outcome after processing all of the events.
enum EventOutcome {
    Continue,
    UpdateQuery(bool),
    Select,
    Quit,
}

/// Process events from the event channel and return the outcome.
fn process_events(
    term: &mut TerminalState,
    events: &Receiver<Event>,
) -> Result<EventOutcome, io::Error> {
    let mut update = false;
    let mut append = true;

    for event in events.try_iter() {
        match event {
            Event::Key(key) => match key.code {
                KeyCode::Char(ch) => {
                    update = true;
                    term.push_char(ch);
                }
                KeyCode::Enter => return Ok(EventOutcome::Select),
                KeyCode::Up => {
                    term.incr_selection();
                }
                KeyCode::Down => {
                    update = true;
                    term.decr_selection();
                }
                KeyCode::Backspace => {
                    update = true;
                    append = false;
                    term.del_char();
                }
                _ => return Ok(EventOutcome::Quit),
            },
            Event::Resize(width, height) => {
                term.resize(width, height);
            }
            _ => {}
        }
    }
    Ok(if update {
        EventOutcome::UpdateQuery(append)
    } else {
        EventOutcome::Continue
    })
}

/// The core picker struct.
///
/// Internally, it holds a [`Nucleo`] instance which is created on initialization.
pub struct Picker<T: Send + Sync + 'static> {
    matcher: Nucleo<T>,
}

impl<T: Send + Sync + 'static> Picker<T> {
    /// Create a new [`Picker`] instance with the prescribed number of columns.
    pub fn new(columns: u32) -> Self {
        Self {
            matcher: Nucleo::new(Config::DEFAULT, std::sync::Arc::new(|| {}), None, columns),
        }
    }

    /// Get an [`Injector`] from the internal [`Nucleo`] instance.
    pub fn injector(&self) -> Injector<T> {
        self.matcher.injector()
    }

    /// Open the picker prompt and return the picked item, if any.
    pub fn pick(&mut self) -> Result<Option<&T>, io::Error> {
        // read keyboard events from a separate thread to avoid 'read()' polling
        // and allow handling multiple keyboard events per frame
        let (sender, receiver) = unbounded();
        thread::spawn(move || loop {
            if let Ok(event) = read() {
                if sender.send(event).is_err() {
                    break;
                }
            }
        });

        pick_internal(&mut self.matcher, receiver, Duration::from_millis(15))
    }
}

fn pick_internal<T: Send + Sync + 'static>(
    matcher: &mut Nucleo<T>,
    events: Receiver<Event>,
    interval: Duration,
) -> Result<Option<&T>, io::Error> {
    let mut stdout = io::stdout();
    let mut term = TerminalState::new(size()?);

    enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;

    let selection = loop {
        let deadline = Instant::now() + interval;

        // increment the matcher and terminal state
        matcher.tick(10);
        term.update_draw_count(matcher.snapshot());

        // process any queued keyboard events and reset query pattern if necessary
        match process_events(&mut term, &events)? {
            EventOutcome::Continue => {}
            EventOutcome::UpdateQuery(append) => {
                matcher.pattern.reparse(
                    0,
                    &term.query,
                    nucleo::pattern::CaseMatching::Smart,
                    nucleo::pattern::Normalization::Smart,
                    append,
                );
            }
            EventOutcome::Select => {
                break term
                    .selector_index
                    .and_then(|idx| matcher.snapshot().get_matched_item(idx as u32))
                    .map(|it| it.data);
            }
            EventOutcome::Quit => {
                break None;
            }
        };

        // redraw the screen
        term.draw(&mut stdout, matcher.snapshot())?;

        // wait before attempting redraw
        thread::sleep(deadline - Instant::now());
    };

    drop(events);
    disable_raw_mode()?;
    execute!(stdout, DisableBracketedPaste, LeaveAlternateScreen)?;
    Ok(selection)
}
