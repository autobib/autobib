use std::io::Write;

use ramify::{Config, Generator, branch_writer, writer::RoundedCornersWide};

use crate::{
    db::{
        state::{InRecordsTable, State},
        tree::RamifierConfig,
    },
    output::stdout_lock_wrap,
};

branch_writer! {
    pub struct InvertedStyle {
        charset: ["│", "─", "╯", "╰",  "╮", "╭", "┤", "├", "┴", "┼"],
        gutter_width: 1,
        inverted: true,
    }
}

fn init_config<S>() -> Config<S> {
    let mut config = Config::new();
    config.row_padding = 2;
    config.annotation_margin = 2;
    config
}

pub fn print_log<'conn, I: InRecordsTable>(
    no_interactive: bool,
    state: &State<'conn, I>,
    tree: bool,
    all: bool,
    reverse: bool,
    oneline: bool,
) -> anyhow::Result<()> {
    let mut stdout = stdout_lock_wrap();
    let styled = !no_interactive && stdout.supports_styled_output();
    let ramifier_config = RamifierConfig {
        all,
        oneline,
        styled,
    };

    // FIXME: copy and paste less here.
    // probably needs a macro because of dependent type weirdness, especially once
    // `oneline` also is implemented which will require a different style, again
    if tree {
        let root = state.current()?.root(all)?;
        let ramifier = state.full_history_ramifier(ramifier_config);

        if reverse {
            let config = init_config::<RoundedCornersWide>();
            let mut generator = Generator::init(root, ramifier, config);

            while generator.try_write_vertex(&mut stdout)? {}
        } else {
            let config = init_config::<InvertedStyle>();
            let mut generator = Generator::init(root, ramifier, config);

            let mut branch_diagram = String::new();
            while generator.try_write_vertex_str(&mut branch_diagram)? {}
            for line in branch_diagram.lines().rev() {
                writeln!(&mut stdout, "{line}")?;
            }
        }
    } else {
        let current = state.current()?;
        let ramifier = state.ancestor_ramifier(ramifier_config);

        if reverse {
            let config = init_config::<InvertedStyle>();
            let mut generator = Generator::init(current, ramifier, config);
            let mut branch_diagram = String::new();

            while generator.try_write_vertex_str(&mut branch_diagram)? {}

            for line in branch_diagram.lines().rev() {
                writeln!(&mut stdout, "{line}")?;
            }
        } else {
            let config = init_config::<RoundedCornersWide>();
            let mut generator = Generator::init(current, ramifier, config);
            while generator.try_write_vertex(&mut stdout)? {}
        }
    }
    Ok(())
}
