use serde::{Deserialize, de::Visitor};

use super::super::{EntryType, MutableEntryData, RecordDataError};

impl TryFrom<Entry> for MutableEntryData {
    type Error = RecordDataError;

    fn try_from(value: Entry) -> Result<Self, Self::Error> {
        let Entry {
            contributors,
            document_type,
            source,
            links,
            title,
            id,
            language,
            database,
            identifier,
            ..
        } = value;

        let entry_type = document_type.code.entry_type();
        let mut record_data = Self::new(entry_type);

        // authors
        let mut author_buf = String::new();
        for author in contributors.authors {
            if author_buf.is_empty() {
                author_buf = author.name;
            } else {
                author_buf.push_str(" and ");
                author_buf.push_str(&author.name);
            }
        }
        if !author_buf.is_empty() {
            record_data.check_and_insert("author".into(), author_buf)?;
        }

        // editors
        let mut editor_buf = String::new();
        for editor in contributors.editors {
            if editor_buf.is_empty() {
                editor_buf = editor.name;
            } else {
                editor_buf.push_str(" and ");
                editor_buf.push_str(&editor.name);
            }
        }
        if !editor_buf.is_empty() {
            record_data.check_and_insert("editor".into(), editor_buf)?;
        }

        // language
        let mut lang_buf = String::new();
        for lang in language.languages {
            if lang_buf.is_empty() {
                lang_buf = lang;
            } else {
                lang_buf.push_str(", ");
                lang_buf.push_str(&lang);
            }
        }
        if !lang_buf.is_empty() {
            record_data.check_and_insert("language".into(), lang_buf)?;
        }

        // zbmath, zbl, jfm keys
        record_data.check_and_insert("zbmath".into(), format!("{id:0>8}"))?;
        if let Some(s) = identifier {
            record_data.check_and_insert(database.as_bibtex().into(), s)?;
        }

        // links, like 'arxiv' and 'doi'
        for link in links {
            if let Some(ty) = link.link_type.as_bibtex() {
                record_data.check_and_insert(ty.into(), link.identifier)?;
            }
        }

        // title parts
        record_data.check_and_insert_if_non_null("titleaddon", title.addition)?;
        record_data.check_and_insert_if_non_null("subtitle", title.subtitle)?;
        record_data.check_and_insert_if_non_null("origtitle", title.original)?;
        record_data.check_and_insert_if_non_null("title", title.title)?;

        // publication details, prioritizing 'series' data more
        if let Some(p) = source.pages {
            match p {
                Pages::Total {
                    _frontmatter: _,
                    total,
                } => {
                    record_data.check_and_insert("pagetotal".into(), total.to_string())?;
                }
                Pages::Range { start, end } => {
                    record_data.check_and_insert("pages".into(), format!("{start}--{end}"))?;
                }
                Pages::Other(s) => {
                    record_data.check_and_insert("pages".into(), s)?;
                }
            }
        }

        for book in source.book {
            record_data.check_and_insert_if_non_null("publisher", book.publisher)?;
            record_data.check_and_insert_if_non_null("year", book.year)?;
        }
        for ser in source.series {
            record_data.check_and_insert_if_non_null("issue", ser.issue)?;
            record_data.check_and_insert_if_non_null("publisher", ser.publisher)?;
            record_data.check_and_insert_if_non_null("journal", ser.short_title)?;
            record_data.check_and_insert_if_non_null("volume", ser.volume)?;
            record_data.check_and_insert_if_non_null("year", ser.year)?;
        }

        Ok(record_data)
    }
}

#[derive(Deserialize)]
pub struct Response {
    pub result: Entry,
    // pub status: Status,
}

// #[derive(Deserialize)]
// pub struct Status {
//     execution: String,
//     execution_bool: bool,
//     nr_total_results: u64,
//     nr_request_results: u64,
//     time_stamp: DateTime<Local>,
// }

#[derive(Deserialize)]
pub struct Entry {
    contributors: Contributors,
    database: Database,
    // datestamp: DateTime<Local>,
    document_type: DocumentType,
    /// The internal zbMath ID
    id: u32,
    /// The Zbl / Jfm identifier, which might not be set for new entries
    identifier: Option<String>,
    language: Language,
    links: Vec<Link>,
    source: Source,
    title: Title,
    // year: Option<String>,
}

#[derive(Deserialize, Clone)]
struct Language {
    languages: Vec<String>,
}

#[derive(Deserialize, Clone, Copy)]
pub enum Database {
    #[serde(rename = "Zbl")]
    Zbl,
    #[serde(rename = "JFM")]
    Jfm,
}

impl Database {
    pub fn as_bibtex(self) -> &'static str {
        match self {
            Self::Zbl => "zbl",
            Self::Jfm => "jfm",
        }
    }
}

#[derive(Deserialize)]
pub struct DocumentType {
    code: Code,
    // description: String,
}

#[derive(Deserialize, Clone, Copy)]
pub enum Code {
    #[serde(rename = "a")]
    CollectionArticle,
    #[serde(rename = "b")]
    Book,
    #[serde(rename = "j")]
    JournalArticle,
}

impl Code {
    fn entry_type(self) -> EntryType {
        match self {
            Self::CollectionArticle => EntryType::in_collection(),
            Self::Book => EntryType::book(),
            Self::JournalArticle => EntryType::article(),
        }
    }
}

#[derive(Deserialize)]
pub struct Title {
    addition: Option<String>,
    original: Option<String>,
    subtitle: Option<String>,
    title: Option<String>,
}

#[derive(Deserialize)]
pub struct Source {
    book: Vec<Book>,
    pages: Option<Pages>,
    series: Vec<Series>,
}

#[derive(Deserialize)]
pub struct Book {
    publisher: Option<String>,
    year: Option<String>,
}

#[derive(Deserialize)]
pub struct Series {
    issue: Option<String>,
    publisher: Option<String>,
    short_title: Option<String>,
    // title: Option<String>,
    volume: Option<String>,
    year: Option<String>,
}

#[derive(Deserialize)]
pub struct Contributors {
    authors: Vec<Author>,
    editors: Vec<Author>,
}

#[derive(Deserialize)]
pub struct Author {
    name: String,
}

#[derive(Deserialize)]
pub struct Link {
    identifier: String,
    #[serde(rename = "type")]
    link_type: LinkType,
    // url: String,
}

#[derive(Deserialize, Clone, Copy)]
pub enum LinkType {
    #[serde(rename = "arxiv")]
    Arxiv,
    #[serde(rename = "doi")]
    Doi,
    #[serde(rename = "https")]
    Https,
    #[serde(rename = "http")]
    Http,
    #[serde(rename = "euclid")]
    Euclid,
}

impl LinkType {
    pub fn as_bibtex(self) -> Option<&'static str> {
        match self {
            Self::Arxiv => Some("arxiv"),
            Self::Doi => Some("doi"),
            _ => None,
        }
    }
}

pub enum Pages {
    Total { _frontmatter: String, total: u64 },
    Range { start: u64, end: u64 },
    Other(String),
}

impl<'de> Deserialize<'de> for Pages {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct PageVisitor;

        impl<'de> Visitor<'de> for PageVisitor {
            type Value = Pages;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("A page count")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                // basically, pages can either be in the format
                // "xv, 531~p." indicating a page count, or
                // "100-200" indicating a range
                // any other format should just be put into `String`
                match value.split_once(", ") {
                    Some((l, pages)) => match pages.strip_suffix("~p.") {
                        Some(page_ct) => {
                            let total: u64 = page_ct.parse().map_err(E::custom)?;
                            Ok(Pages::Total {
                                _frontmatter: l.into(),
                                total,
                            })
                        }
                        None => Ok(Pages::Other(value.into())),
                    },
                    None => match value.split_once("-") {
                        Some((l, r)) => {
                            let start: u64 = l.parse().map_err(E::custom)?;
                            let end: u64 = r.parse().map_err(E::custom)?;
                            Ok(Pages::Range { start, end })
                        }
                        None => Ok(Pages::Other(value.into())),
                    },
                }
            }
        }

        deserializer.deserialize_str(PageVisitor)
    }
}
