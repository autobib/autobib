use std::io::Write;

use anyhow::Result;
use nucleo_picker::{Interface, Picker, Render};

use crate::{db::state::Record, logger::error, output::stdout_lock_wrap};

pub fn pick_and_output(
    picker: &mut Picker<Record, impl Render<Record>, impl Interface<Record>>,
) -> Result<()> {
    let selection = picker.pick()?.into_iter();
    if selection.len() == 0 {
        error!("No item selected.");
    } else {
        let mut stdout = stdout_lock_wrap();
        for row_data in selection {
            writeln!(&mut stdout, "{}", row_data.canonical)?;
        }
    }

    Ok(())
}
