use std::{
    path::{Path, PathBuf},
    thread,
};

use nonempty::NonEmpty;
use nucleo_picker::{Picker, PickerOptions, Render};
use walkdir::{DirEntry, WalkDir};

use crate::{
    db::{RecordDatabase, state::RecordRow},
    entry::RawEntryData,
    format::Template,
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
    template: Template,
    strict: bool,
    attachment_root: PathBuf,
    ignore_hidden: bool,
    mut filter: F,
) -> Picker<AttachmentData, Template> {
    // initialize picker
    let picker = Picker::new(template);

    // populate the picker from a separate thread
    let injector = picker.injector();
    thread::spawn(move || {
        record_db.inject_active_records(injector.clone(), |row_data| {
            if strict && !injector.renderer().has_keys_contained_in(&row_data) {
                return None;
            }

            // fill the buffer with the attachment path
            let mut attachment_root = attachment_root.to_path_buf();
            row_data
                .canonical
                .extend_attachments_path(&mut attachment_root);

            // walk through all of the entries in the attachment path
            let paths = if ignore_hidden {
                fn is_hidden(entry: &DirEntry) -> bool {
                    entry
                        .file_name()
                        .to_str()
                        .map(|s| s.starts_with("."))
                        .unwrap_or(false)
                }

                NonEmpty::collect(
                    WalkDir::new(&attachment_root)
                        .into_iter()
                        .filter_entry(|e| !is_hidden(e))
                        .flatten()
                        .filter(|dir_entry| filter(dir_entry.path())),
                )
            } else {
                NonEmpty::collect(
                    WalkDir::new(&attachment_root)
                        .into_iter()
                        .flatten()
                        .filter(|dir_entry| filter(dir_entry.path())),
                )
            };
            paths.map(|attachments| AttachmentData {
                row_data,
                attachments,
                attachment_root,
            })
        })
    });

    picker
}

/// Returns a picker which returns the record data associated with the picked item.
#[allow(clippy::type_complexity)]
pub fn choose_canonical_id(
    mut record_db: RecordDatabase,
    template: Template,
    strict: bool,
) -> (
    Picker<RecordRow<RawEntryData>, Template>,
    thread::JoinHandle<Result<RecordDatabase, rusqlite::Error>>,
) {
    // initialize picker
    let picker = Picker::new(template);

    // populate the picker from a separate thread
    let injector = picker.injector();
    let handle = thread::spawn(move || {
        // TODO: to better support cancellation here, we could use an Arc<AtomicBool>
        // cancellation token; paginate the select using `SELECT ... LIMIT ...` with some sane
        // page size (maybe 10k? this should take <1ms per page), and then check for cancellation
        // between pages.
        record_db.inject_active_records(injector.clone(), |row_data| {
            if strict && !injector.renderer().has_keys_contained_in(&row_data) {
                None
            } else {
                Some(row_data)
            }
        })?;
        Ok(record_db)
    });

    (picker, handle)
}

/// A wrapper around a [`RecordRow`] which also contains a list of attachments associated with the
/// record.
pub struct AttachmentData {
    pub row_data: RecordRow<RawEntryData>,
    pub attachments: NonEmpty<DirEntry>,
    pub attachment_root: PathBuf,
}

impl Render<AttachmentData> for Template {
    type Str<'a> = String;

    fn render<'a>(&self, item: &'a AttachmentData) -> Self::Str<'a> {
        self.render(&item.row_data)
    }
}
