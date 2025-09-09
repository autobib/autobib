use chrono::{DateTime, FixedOffset};
use rsxiv::{
    id::{ArticleId, normalize},
    response::{AuthorName, Response},
};
use serde::Deserialize;

use super::{
    Client, EntryType, ProviderError, RecordData, RecordDataError, Response as _, StatusCode,
    ValidationOutcome,
};

pub fn is_valid_id(id: &str) -> ValidationOutcome {
    match normalize(id) {
        Ok(Some((l, r))) => {
            let mut s = String::with_capacity(l.len() + r.len());
            s.push_str(l);
            s.push_str(r);
            ValidationOutcome::Normalize(s)
        }
        Ok(None) => ValidationOutcome::Valid,
        Err(_) => ValidationOutcome::Invalid,
    }
}

#[derive(Deserialize)]
struct Entry {
    id: ArticleId,
    updated: DateTime<FixedOffset>,
    published: DateTime<FixedOffset>,
    authors: Vec<AuthorName>,
    title: String,
    doi: Option<String>,
}

impl TryFrom<Entry> for RecordData {
    type Error = RecordDataError;

    fn try_from(entry: Entry) -> Result<Self, Self::Error> {
        let mut record_data = Self::new(EntryType::preprint());

        let Entry {
            id,
            updated,
            published,
            authors,
            title,
            doi,
        } = entry;

        let mut author_buf = String::new();
        for AuthorName {
            keyname,
            firstnames,
            suffix,
        } in authors
        {
            if !author_buf.is_empty() {
                author_buf.push_str(" and ");
            }

            // format like `von Last, Jr, First`
            author_buf.push_str(&keyname);
            if !suffix.is_empty() {
                author_buf.push_str(", ");
                author_buf.push_str(&suffix);
            }
            if !firstnames.is_empty() {
                author_buf.push_str(", ");
                author_buf.push_str(&firstnames);
            }
        }

        // TODO: capture `updated` data here in date as well as date handling, but this should wait
        // until `date` normalization exists
        record_data.check_and_insert("arxiv".into(), id.to_string())?;
        record_data.check_and_insert("author".into(), author_buf)?;
        // record_data.check_and_insert("date".into(), updated.format("%Y-%m-%d").to_string())?;
        if let Some(s) = doi {
            record_data.check_and_insert("doi".into(), s.trim().to_owned())?;
        }
        record_data.check_and_insert("month".into(), updated.format("%m").to_string())?;
        record_data
            .check_and_insert("origdate".into(), published.format("%Y-%m-%d").to_string())?;
        record_data.check_and_insert("title".into(), title.trim().to_owned())?;
        if let Some(v) = id.version() {
            record_data.check_and_insert("version".into(), v.to_string())?;
        }
        record_data.check_and_insert("year".into(), updated.format("%Y").to_string())?;

        Ok(record_data)
    }
}

pub fn get_record<C: Client>(id: &str, client: &C) -> Result<Option<RecordData>, ProviderError> {
    let mut response = client.get(format!("https://export.arxiv.org/api/query?id_list={id}"))?;

    let body = match response.status() {
        StatusCode::OK => response.bytes()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(ProviderError::UnexpectedStatusCode(code)),
    };

    match Response::<Option<Entry>>::from_xml(&body) {
        Ok(response) => match response.entries {
            Some(entry) => Ok(Some(entry.try_into()?)),
            None => Ok(None),
        },
        Err(err) => Err(ProviderError::Unexpected(format!(
            "arXiv XML response had an unexpected format! Response body:\n{}\nError message:\n{err}",
            String::from_utf8_lossy(&body)
        ))),
    }
}
