use std::{
    cmp::Ordering,
    path::{Path, PathBuf},
    thread,
};

use lockstep::{Control, lockstep};
use nonempty::NonEmpty;
use nucleo_picker::{Picker, PickerOptions, Render};
use walkdir::{DirEntry, WalkDir};

use crate::{
    db::{RecordDatabase, state::RowData},
    entry::EntryData,
    path_hash::PathHash,
};

pub struct DirEntryRenderer {
    root: PathBuf,
}

impl Render<DirEntry> for DirEntryRenderer {
    type Str<'a> = std::borrow::Cow<'a, str>;

    fn render<'a>(&self, item: &'a DirEntry) -> Self::Str<'a> {
        item.path()
            .strip_prefix(&self.root)
            .expect("DirEntry was created originally from this root path")
            .to_string_lossy()
    }
}

pub fn choose_attachment(att_data: &AttachmentData) -> Picker<DirEntry, DirEntryRenderer> {
    let mut picker = PickerOptions::new()
        .config(nucleo_picker::nucleo::Config::DEFAULT.match_paths())
        // Use our custom renderer for a `DirEntry`
        .picker(DirEntryRenderer {
            root: att_data.attachment_root.clone(),
        });

    picker.extend(att_data.attachments.iter().cloned());

    picker
}

/// Returns a picker which returns the record attachment data associated with the picked item.
pub fn choose_attachment_path<F: FnMut(&Path) -> bool + Send + 'static>(
    mut record_db: RecordDatabase,
    fields_to_search: Vec<String>,
    entry_type: bool,
    all_fields: bool,
    attachment_root: PathBuf,
    mut filter: F,
) -> Picker<AttachmentData, FieldFilterRenderer> {
    // initialize picker
    let picker = Picker::new(FieldFilterRenderer::new(
        fields_to_search,
        " ~ ",
        entry_type,
    ));

    // populate the picker from a separate thread
    let injector = picker.injector();
    thread::spawn(move || {
        record_db.inject_records(injector.clone(), |row_data| {
            if (all_fields && !injector.renderer().all_match(&row_data))
                || (!all_fields && !injector.renderer().any_match(&row_data))
            {
                return None;
            }

            // fill the buffer with the attachment path
            let mut attachment_root = attachment_root.to_path_buf();
            row_data
                .canonical
                .extend_attachments_path(&mut attachment_root);

            // walk through all of the entries in the attachment path
            NonEmpty::collect(
                WalkDir::new(&attachment_root)
                    .into_iter()
                    .flatten()
                    .filter(|dir_entry| filter(dir_entry.path())),
            )
            .map(|attachments| AttachmentData {
                row_data,
                attachments,
                attachment_root,
            })
        })
    });

    picker
}

/// Returns a picker which returns the record data associated with the picked item.
pub fn choose_canonical_id(
    mut record_db: RecordDatabase,
    fields_to_search: Vec<String>,
    entry_type: bool,
    all_fields: bool,
) -> (
    Picker<RowData, FieldFilterRenderer>,
    thread::JoinHandle<Result<RecordDatabase, rusqlite::Error>>,
) {
    // initialize picker
    let picker = Picker::new(FieldFilterRenderer::new(
        fields_to_search,
        " ~ ",
        entry_type,
    ));

    // populate the picker from a separate thread
    let injector = picker.injector();
    let handle = thread::spawn(move || {
        // TODO: to better support cancellation here, we could use an Arc<AtomicBool>
        // cancellation token; paginate the select using `SELECT ... LIMIT ...` with some sane
        // page size (maybe 10k? this should take <1ms per page), and then check for cancellation
        // between pages.
        record_db.inject_records(injector.clone(), |row_data| {
            if (all_fields && !injector.renderer().all_match(&row_data))
                || (!all_fields && !injector.renderer().any_match(&row_data))
            {
                None
            } else {
                Some(row_data)
            }
        })?;
        Ok(record_db)
    });

    (picker, handle)
}

/// A wrapper around a [`RowData`] which also contains a list of attachments associated with the
/// record.
pub struct AttachmentData {
    pub row_data: RowData,
    pub attachments: NonEmpty<DirEntry>,
    pub attachment_root: PathBuf,
}

/// Given a set of allowed fields, renders those fields which are present in the
/// data in alphabetical order, separated by the `separator`. If `entry_type` is `true`, also
/// render the entry type as a prefix, for example `article: `.
pub struct FieldFilterRenderer {
    fields_to_search: Vec<String>,
    separator: &'static str,
    entry_type: bool,
}

impl FieldFilterRenderer {
    /// Returns a new renderer. The `fields_to_search` are normalized to be ASCII lowercase, and
    /// then sorted and deduplicated.
    fn new(mut fields_to_search: Vec<String>, separator: &'static str, entry_type: bool) -> Self {
        fields_to_search
            .iter_mut()
            .for_each(|s| s.make_ascii_lowercase());
        fields_to_search.sort();
        fields_to_search.dedup();

        Self {
            fields_to_search,
            separator,
            entry_type,
        }
    }

    /// Returns an iterator over the values in the data corresponding to the `row_data` which also
    /// appear as values in the provided `fields_to_search`.
    #[inline]
    fn iter_intersection<'a>(
        &self,
        row_data: &'a RowData,
    ) -> impl Iterator<Item = &'a str> + use<'_, 'a> {
        lockstep(
            row_data.data.fields(),
            &self.fields_to_search,
            |(key, _), field| match (*key).cmp(field.as_ref()) {
                Ordering::Less => Control::SkipLeft,
                Ordering::Equal => Control::Yield,
                Ordering::Greater => Control::SkipRight,
            },
        )
        .map(|((_, val), _)| val)
    }

    /// Returns if the provided `row_data` contains any key matching a field in the provided search
    /// fields.
    #[inline]
    fn any_match(&self, row_data: &RowData) -> bool {
        self.iter_intersection(row_data).next().is_some()
    }

    /// Returns if every field in the provided search fields also appears in `row_data`.
    #[inline]
    fn all_match(&self, row_data: &RowData) -> bool {
        let mut data_fields = row_data.data.fields().map(|(key, _)| key);

        for field in &self.fields_to_search {
            'inner: loop {
                match data_fields.next() {
                    Some(elem) => match (*elem).cmp(field.as_ref()) {
                        // elem < field, so we just try the next element
                        Ordering::Less => {}
                        // elem == field, so we found the matching element so we move to the next
                        // field
                        Ordering::Equal => break 'inner,
                        // elem > field, so we missed the element so we abort
                        Ordering::Greater => return false,
                    },
                    // no more elements, so we missed the element and we abort
                    None => return false,
                }
            }
        }

        true
    }
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
        for val in self.iter_intersection(row_data) {
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
