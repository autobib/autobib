use std::io::Write;

use ramify::{Config, Generator, branch_writer};

use crate::{
    db::state::{RecordKey, State},
    output::stdout_lock_wrap,
};

branch_writer! {
    pub struct InvertedStyle {
        charset: ["│", "─", "╯", "╰",  "╮", "╭", "┤", "├", "┴", "┼"],
        gutter_width: 1,
        inverted: true,
    }
}

pub fn print_log<'conn>(state: &State<'conn, RecordKey>, all: bool) -> anyhow::Result<()> {
    if all {
        let root = state.root()?;
        let mut config = Config::<InvertedStyle>::new();
        config.row_padding = 2;
        config.annotation_margin = 2;
        let mut generator = Generator::init(root, state.full_history_ramifier(), config);
        let mut branch_diagram = String::new();

        let mut limit = state.changelog_size()?;

        while generator.try_write_vertex_str(&mut branch_diagram)? {
            if limit > 0 {
                limit -= 1;
            } else {
                panic!("Database changelog history contains infinite loop");
            }
        }

        let mut stdout = stdout_lock_wrap();
        for line in branch_diagram.lines().rev() {
            writeln!(&mut stdout, "{line}")?;
        }
    } else {
        let current = state.current()?;
        let mut config = Config::<ramify::writer::RoundedCornersWide>::new();
        config.row_padding = 2;
        config.annotation_margin = 2;
        let mut generator = Generator::init(current, state.ancestor_ramifier(), config);
        let mut stdout = stdout_lock_wrap();
        let mut limit = state.changelog_size()?;

        while generator.try_write_vertex(&mut stdout)? {
            if limit > 0 {
                limit -= 1;
            } else {
                panic!("Database changelog history contains infinite loop");
            }
        }
    }
    Ok(())
}
