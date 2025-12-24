use ramify::{Config, TryRamify, writer::Style};

use crate::{
    db::{
        state::{InRecordsTable, State},
        tree::RamifierConfig,
    },
    output::stdout_lock_wrap,
};

fn write_branch_diagram<V, R, W>(
    mut writer: W,
    root: V,
    ramifier: R,
    invert: bool,
) -> anyhow::Result<()>
where
    R: TryRamify<V>,
    <R as TryRamify<V>>::Error: 'static + Send + Sync + std::error::Error,
    W: std::io::Write,
{
    let config = Config::new().row_padding(2);
    let style = Style::rounded_corners()
        .gutter_width(1)
        .annotation_margin(2);

    if invert {
        let branch_diagram = config
            .inverted_annotations(true)
            .generator(root, ramifier)
            .try_branch_diagram(style.invert())?;

        for line in branch_diagram.lines().rev() {
            writeln!(writer, "{line}")?;
        }
    } else {
        let mut writer = style.io_writer(writer);

        config
            .generator(root, ramifier)
            .try_write_all(&mut writer)?
            .halt_if_suspended()?;
    }
    Ok(())
}

pub fn print_log<'conn, I: InRecordsTable>(
    no_interactive: bool,
    state: &State<'conn, I>,
    tree: bool,
    all: bool,
    reverse: bool,
    oneline: bool,
) -> anyhow::Result<()> {
    let stdout = stdout_lock_wrap();
    let styled = !no_interactive && stdout.supports_styled_output();
    let ramifier_config = RamifierConfig {
        all,
        oneline,
        styled,
    };

    let current = state.current()?;

    if tree {
        let ramifier = state.full_history_ramifier(ramifier_config);
        write_branch_diagram(stdout, current.root(all)?, ramifier, !reverse)
    } else {
        let ramifier = state.ancestor_ramifier(ramifier_config);
        write_branch_diagram(stdout, current, ramifier, reverse)
    }
}
