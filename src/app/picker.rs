use std::{collections::HashSet, path::PathBuf, thread};

use nucleo_picker::{Picker, PickerOptions, Render};
use walkdir::{DirEntry, WalkDir};

use crate::{
    db::{state::RowData, EntryData, RecordDatabase},
    path_hash::PathHash,
};

pub struct DirEntryRenderer;

impl Render<DirEntry> for DirEntryRenderer {
    type Str<'a> = std::borrow::Cow<'a, str>;

    fn render<'a>(&self, item: &'a DirEntry) -> Self::Str<'a> {
        item.path().to_string_lossy()
    }
}

pub fn choose_attachment<'a>(
    attachments: impl IntoIterator<Item = &'a DirEntry>,
) -> Picker<DirEntry, DirEntryRenderer> {
    let mut picker = PickerOptions::new()
        .config(nucleo_picker::nucleo::Config::DEFAULT.match_paths())
        // Use our custom renderer for a `DirEntry`
        .picker(DirEntryRenderer);

    picker.extend(attachments.into_iter().cloned());

    picker
}

/// Returns a picker which returns the record attachment data associated with the picked item.
pub fn choose_attachment_path<F: FnMut(&std::path::Path) -> bool + Send + 'static>(
    mut record_db: RecordDatabase,
    fields_to_search: HashSet<String>,
    entry_type: bool,
    attachment_root: PathBuf,
    mut filter: F,
) -> Picker<AttachmentData, FieldFilterRenderer> {
    // initialize picker
    let picker = Picker::new(FieldFilterRenderer {
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

    picker
}

/// Returns a picker which returns the record data associated with the picked item.
pub fn choose_canonical_id(
    mut record_db: RecordDatabase,
    fields_to_search: HashSet<String>,
    entry_type: bool,
) -> Picker<RowData, FieldFilterRenderer> {
    // initialize picker
    let picker = Picker::new(FieldFilterRenderer {
        fields_to_search,
        separator: " ~ ",
        entry_type,
    });

    // populate the picker from a separate thread
    let injector = picker.injector();
    thread::spawn(move || record_db.inject_all_records(injector));

    picker
}

/// A wrapper around a [`RowData`] which also contains a list of attachments associated with the
/// record.
pub struct AttachmentData {
    pub row_data: RowData,
    pub attachments: Vec<DirEntry>,
}

/// Given a set of allowed fields, renders those fields which are present in the
/// data in alphabetical order, separated by the `separator`. If `entry_type` is `true`, also
/// render the entry type as a prefix, for example `article: `.
pub struct FieldFilterRenderer {
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

impl Render<AttachmentData> for FieldFilterRenderer {
    type Str<'a> = String;

    fn render<'a>(&self, item: &'a AttachmentData) -> Self::Str<'a> {
        self.render(&item.row_data)
    }
}
