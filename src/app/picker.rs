use std::{collections::HashSet, io, thread};

use nucleo_picker::{Picker, Render};

use crate::{
    db::{state::RowData, EntryData, RecordDatabase},
    record::RemoteId,
};

/// Open an interactive prompt for the user to select a record.
pub fn choose_canonical_id(
    mut record_db: RecordDatabase,
    fields_to_search: HashSet<String>,
) -> Result<Option<RemoteId>, io::Error> {
    // initialize picker
    let mut picker = Picker::new(FieldFilterRenderer {
        fields_to_search,
        separator: " ~ ",
    });

    // populate the picker from a separate thread
    let injector = picker.injector();
    thread::spawn(move || record_db.inject_all_records(injector));

    // get the selection
    Ok(picker.pick()?.map(|row_data| row_data.canonical.clone()))
}

/// Given a set of allowed fields renders those fields which are present in the
/// data in alphabetical order, separated by the `separator`.
struct FieldFilterRenderer {
    fields_to_search: HashSet<String>,
    separator: &'static str,
}

impl Render<RowData> for FieldFilterRenderer {
    type Str<'a> = String;

    fn render<'a>(&self, row_data: &'a RowData) -> Self::Str<'a> {
        let mut output: String = row_data.data.entry_type().into();
        output.push_str(": ");

        let mut first = true;
        for (_, val) in row_data
            .data
            .fields()
            .filter(|(key, _)| self.fields_to_search.contains(*key))
        {
            if first {
                first = false;
            } else {
                output.push_str(self.separator);
            }
            output.push_str(val);
        }
        output
    }
}
