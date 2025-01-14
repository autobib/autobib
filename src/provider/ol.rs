use std::sync::LazyLock;

use regex::Regex;
use reqwest::StatusCode;
use serde::Deserialize;

use crate::logger::info;

use super::{HttpClient, ProviderError, RecordData, ValidationOutcome};

static OL_IDENTIFIER_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]{8}M$").unwrap());

pub fn is_valid_id(id: &str) -> ValidationOutcome {
    OL_IDENTIFIER_RE.is_match(id).into()
}

#[derive(Deserialize)]
struct OpenLibraryAuthor {
    name: String,
}

#[derive(Deserialize)]
struct AuthorID {
    key: String,
}

#[derive(Deserialize)]
struct OpenLibraryRecord {
    #[serde(default)]
    authors: Vec<AuthorID>,
    #[allow(unused)]
    full_title: Option<String>,
    edition_name: Option<String>,
    number_of_pages: Option<usize>,
    subtitle: Option<String>,
    #[serde(default)]
    isbn_13: Vec<String>,
    publish_date: Option<String>,
    #[serde(default)]
    publish_places: Vec<String>,
    #[serde(default)]
    publishers: Vec<String>,
    title: Option<String>,
}

pub fn get_record(id: &str, client: &HttpClient) -> Result<Option<RecordData>, ProviderError> {
    let response = client.get(format!("https://openlibrary.org/books/OL{id}.json"))?;

    let body = match response.status() {
        StatusCode::OK => response.bytes()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(ProviderError::UnexpectedStatusCode(code)),
    };

    match serde_json::from_slice(&body) {
        Ok(OpenLibraryRecord {
            authors,
            title,
            edition_name,
            number_of_pages,
            subtitle,
            isbn_13,
            publish_date,
            publish_places,
            publishers,
            ..
        }) => {
            let mut record_data = RecordData::try_new("book".into()).unwrap();

            if let Some(address) = publish_places.into_iter().next() {
                record_data.check_and_insert("address".into(), address)?;
            }

            // we need to make separate requests for the authors
            if !authors.is_empty() {
                let mut auth_string = String::new();
                let mut first = true;

                for AuthorID { key } in authors {
                    info!("Making remote request for OpenLibrary author at {key}");
                    let response = client.get(format!("https://openlibrary.org{key}.json"))?;

                    let body = match response.status() {
                        StatusCode::OK => response.bytes()?,
                        code => return Err(ProviderError::UnexpectedStatusCode(code)),
                    };

                    match serde_json::from_slice(&body) {
                        Ok(OpenLibraryAuthor { name }) => {
                            if first {
                                first = false;
                            } else {
                                auth_string.push_str(" and ");
                            }
                            auth_string.push_str(&name);
                        }
                        Err(err) => {
                            return Err(ProviderError::Unexpected(format!(
                                "Unexpected author data in Open Library record: {err}"
                            )))
                        }
                    }
                }

                record_data.check_and_insert("author".into(), auth_string)?;
            }

            if let Some(date) = publish_date {
                record_data.check_and_insert("date".into(), date)?;
            }

            if let Some(edition) = edition_name {
                record_data.check_and_insert("edition".into(), edition)?;
            }

            if let Some(isbn) = isbn_13.into_iter().next() {
                record_data.check_and_insert("isbn".into(), isbn)?;
            }

            if let Some(page_count) = number_of_pages {
                record_data.check_and_insert("pagetotal".into(), page_count.to_string())?;
            }

            if !publishers.is_empty() {
                record_data.check_and_insert("publisher".into(), publishers.join(" and "))?;
            }

            if let Some(subtitle) = subtitle {
                record_data.check_and_insert("subtitle".into(), subtitle)?;
            }

            if let Some(title) = title {
                record_data.check_and_insert("title".into(), title)?;
            }
            Ok(Some(record_data))
        }
        Err(err) => Err(ProviderError::Unexpected(err.to_string())),
    }
}
