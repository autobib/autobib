use std::{collections::HashSet, io, path::PathBuf, thread};

use nucleo_picker::{Picker, Render};
use walkdir::{DirEntry, WalkDir};

use crate::{
    db::{state::RowData, EntryData, RecordDatabase},
    path_hash::PathHash,
    record::RemoteId,
};

pub fn choose_attachment_path<F: FnMut(&std::path::Path) -> bool + Send + 'static>(
    mut record_db: RecordDatabase,
    fields_to_search: HashSet<String>,
    entry_type: bool,
    attachment_root: PathBuf,
    mut filter: F,
) -> Result<Option<Vec<DirEntry>>, io::Error> {
    // initialize picker
    let mut picker = Picker::new(FieldFilterRenderer {
        fields_to_search,
        separator: " ~ ",
        entry_type,
    });

    // populate the picker from a separate thread
    let injector = picker.injector();
    thread::spawn(move || {
        // save some alloctions by reusing the underlying buffer for the temp 'walk dir' root
        let mut buffer = PathBuf::new();

        record_db.inject_records(injector, |row_data| {
            // fill the buffer with the attachment path
            buffer.clone_from(&attachment_root);
            row_data.canonical.extend_attachments_path(&mut buffer);

            // walk through all of the entries in the attachment path
            let attachments: Vec<_> = WalkDir::new(&buffer)
                .into_iter()
                .flatten()
                .filter(|dir_entry| filter(dir_entry.path()))
                .collect();

            if attachments.is_empty() {
                None
            } else {
                Some(AttachmentData {
                    row_data,
                    attachments,
                })
            }
        })
    });

    // get the selection
    Ok(picker.pick()?.map(|data| data.attachments.clone()))
}

/// Open an interactive prompt for the user to select a record.
pub fn choose_canonical_id(
    mut record_db: RecordDatabase,
    fields_to_search: HashSet<String>,
    entry_type: bool,
) -> Result<Option<RemoteId>, io::Error> {
    // initialize picker
    let mut picker = Picker::new(FieldFilterRenderer {
        fields_to_search,
        separator: " ~ ",
        entry_type,
    });

    // populate the picker from a separate thread
    let injector = picker.injector();
    thread::spawn(move || record_db.inject_all_records(injector));

    // get the selection
    Ok(picker.pick()?.map(|row_data| row_data.canonical.clone()))
}

struct AttachmentData {
    row_data: RowData,
    attachments: Vec<DirEntry>,
}

impl Render<AttachmentData> for FieldFilterRenderer {
    type Str<'a> = String;

    fn render<'a>(&self, item: &'a AttachmentData) -> Self::Str<'a> {
        self.render(&item.row_data)
    }
}

/// Given a set of allowed fields renders those fields which are present in the
/// data in alphabetical order, separated by the `separator`.
struct FieldFilterRenderer {
    fields_to_search: HashSet<String>,
    separator: &'static str,
    entry_type: bool,
}

impl Render<RowData> for FieldFilterRenderer {
    type Str<'a> = String;

    fn render<'a>(&self, row_data: &'a RowData) -> Self::Str<'a> {
        let mut output = if self.entry_type {
            row_data.data.entry_type().to_owned() + ": "
        } else {
            String::new()
        };

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
