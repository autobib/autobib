use std::sync::LazyLock;

use chrono::NaiveDate;
use itertools::Itertools;
use regex::Regex;
use reqwest::StatusCode;
use serde::Deserialize;

use super::{HttpClient, ProviderError, RecordData, RecordDataError};

static ARXIV_IDENTIFIER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
            r"^(",
            // old style:
            r"(",
            // first, the archive
            r"(acc-phys|adap-org|alg-geom|ao-sci|astro-ph|astro-ph\.CO|astro-ph\.EP|astro-ph\.GA|astro-ph\.HE|astro-ph\.IM|astro-ph\.SR|atom-ph|bayes-an|chao-dyn|chem-ph|cmp-lg|comp-gas|cond-mat|cond-mat\.dis-nn|cond-mat\.mes-hall|cond-mat\.mtrl-sci|cond-mat\.other|cond-mat\.quant-gas|cond-mat\.soft|cond-mat\.stat-mech|cond-mat\.str-el|cond-mat\.supr-con|cs\.AI|cs\.AR|cs\.CC|cs\.CE|cs\.CG|cs\.CL|cs\.CR|cs\.CV|cs\.CY|cs\.DB|cs\.DC|cs\.DL|cs\.DM|cs\.DS|cs\.ET|cs\.FL|cs\.GL|cs\.GR|cs\.GT|cs\.HC|cs\.IR|cs\.IT|cs\.LG|cs\.LO|cs\.MA|cs\.MM|cs\.MS|cs\.NA|cs\.NE|cs\.NI|cs\.OH|cs\.OS|cs\.PF|cs\.PL|cs\.RO|cs\.SC|cs\.SD|cs\.SE|cs\.SI|cs\.SY|dg-ga|econ\.EM|econ\.GN|econ\.TH|eess\.AS|eess\.IV|eess\.SP|eess\.SY|funct-an|gr-qc|hep-ex|hep-lat|hep-ph|hep-th|math-ph|math\.AC|math\.AG|math\.AP|math\.AT|math\.CA|math\.CO|math\.CT|math\.CV|math\.DG|math\.DS|math\.FA|math\.GM|math\.GN|math\.GR|math\.GT|math\.HO|math\.IT|math\.KT|math\.LO|math\.MG|math\.MP|math\.NA|math\.NT|math\.OA|math\.OC|math\.PR|math\.QA|math\.RA|math\.RT|math\.SG|math\.SP|math\.ST|mtrl-th|nlin\.AO|nlin\.CD|nlin\.CG|nlin\.PS|nlin\.SI|nucl-ex|nucl-th|patt-sol|physics\.acc-ph|physics\.ao-ph|physics\.app-ph|physics\.atm-clus|physics\.atom-ph|physics\.bio-ph|physics\.chem-ph|physics\.class-ph|physics\.comp-ph|physics\.data-an|physics\.ed-ph|physics\.flu-dyn|physics\.gen-ph|physics\.geo-ph|physics\.hist-ph|physics\.ins-det|physics\.med-ph|physics\.optics|physics\.plasm-ph|physics\.pop-ph|physics\.soc-ph|physics\.space-ph|plasm-ph|q-alg|q-bio|q-bio\.BM|q-bio\.CB|q-bio\.GN|q-bio\.MN|q-bio\.NC|q-bio\.OT|q-bio\.PE|q-bio\.QM|q-bio\.SC|q-bio\.TO|q-fin\.CP|q-fin\.EC|q-fin\.GN|q-fin\.MF|q-fin\.PM|q-fin\.PR|q-fin\.RM|q-fin\.ST|q-fin\.TR|quant-ph|solv-int|stat\.AP|stat\.CO|stat\.ME|stat\.ML|stat\.OT|stat\.TH|supr-con)",
            r"/",
            // followed by the identifier
            r"(([0-1][0-9])|(9[1-9]))(0[1-9]|1[0-2])([0-9]{3})",
            r")",
            r"|",
            // new style: YYMM.NNNN or YYMM.NNNNN
            r"([0-9][0-9](0[1-9]|1[0-2])[.][0-9]{4,5})",
            r")$",
    )).unwrap()
});

pub fn is_valid_id(id: &str) -> bool {
    ARXIV_IDENTIFIER_RE.is_match(id)
}

#[derive(Deserialize, Debug)]
struct ArxivXMLDe {
    #[serde(rename = "GetRecord")]
    response: Option<ArxivResponse>,
    error: Option<ArxivError>,
}

#[allow(dead_code)]
#[derive(Debug)]
enum ArxivXML {
    Response(ArxivResponse),
    Error(ArxivError),
}

impl TryInto<ArxivXML> for ArxivXMLDe {
    type Error = ProviderError;

    fn try_into(self) -> Result<ArxivXML, Self::Error> {
        match self {
            Self {
                response: Some(resp),
                error: None,
            } => Ok(ArxivXML::Response(resp)),
            Self {
                response: None,
                error: Some(err),
            } => Ok(ArxivXML::Error(err)),
            _ => Err(Self::Error::Unexpected(
                "arXiv XML response had an unexpected format!".into(),
            )),
        }
    }
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct ArxivError {
    #[serde(rename = "@code")]
    code: ArxivErrorCode,
    #[serde(rename = "$value")]
    message: String,
}

#[derive(Deserialize, Debug)]
enum ArxivErrorCode {
    #[serde(rename = "idDoesNotExist")]
    Missing,
}

#[derive(Deserialize, Debug)]
struct ArxivResponse {
    record: ArxivRecord,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct ArxivRecord {
    header: ArxivHeader,
    metadata: ArxivMetadata,
}

#[derive(Deserialize, Debug)]
struct ArxivMetadata {
    #[serde(rename = "arXiv")]
    contents: ArxivEntry,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct ArxivHeader {
    identifier: String,
    datestamp: NaiveDate,
    #[serde(rename = "setSpec")]
    spec: Vec<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
struct ArxivEntry {
    id: String,
    created: NaiveDate,
    updated: Option<NaiveDate>,
    license: String,
    doi: Option<String>,
    authors: ArxivAuthorList,
    title: String,
    #[serde(rename = "abstract")]
    abs: String,
}

#[derive(Deserialize, Debug)]
struct ArxivAuthorList {
    author: Vec<ArxivAuthor>,
}

#[derive(Deserialize, Debug)]
struct ArxivAuthor {
    keyname: String,
    forenames: String,
}

impl std::fmt::Display for ArxivAuthor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}, {}", self.keyname.trim(), self.forenames.trim())
    }
}

impl std::fmt::Display for ArxivAuthorList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.author.iter().join(" and "))
    }
}

impl TryFrom<ArxivResponse> for RecordData {
    type Error = RecordDataError;

    fn try_from(arxiv_response: ArxivResponse) -> Result<Self, Self::Error> {
        let mut record_data = RecordData::try_new("preprint".into()).unwrap();

        let ArxivResponse {
            record:
                ArxivRecord {
                    metadata:
                        ArxivMetadata {
                            contents:
                                ArxivEntry {
                                    created,
                                    authors,
                                    title,
                                    id,
                                    doi,
                                    ..
                                },
                        },
                    ..
                },
        } = arxiv_response;

        record_data.check_and_insert("arxiv".into(), id.trim().to_owned())?;
        record_data.check_and_insert("author".into(), authors.to_string())?;
        if let Some(s) = doi {
            record_data.check_and_insert("doi".into(), s.trim().to_owned())?;
        }
        record_data.check_and_insert("month".into(), created.format("%m").to_string())?;
        record_data.check_and_insert("title".into(), title.trim().to_owned())?;
        record_data.check_and_insert("year".into(), created.format("%Y").to_string())?;

        Ok(record_data)
    }
}

pub fn get_record(id: &str, client: &HttpClient) -> Result<Option<RecordData>, ProviderError> {
    let response = client.get(format!(
        "https://export.arxiv.org/oai2?verb=GetRecord&identifier=oai:arXiv.org:{id}&metadataPrefix=arXiv"
    ))?;

    let body = match response.status() {
        StatusCode::OK => response.text()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(ProviderError::UnexpectedStatusCode(code)),
    };

    match quick_xml::de::from_str::<ArxivXMLDe>(&body) {
        Ok(parsed) => match parsed.try_into()? {
            ArxivXML::Response(response) => Ok(Some(response.try_into()?)),
            ArxivXML::Error(_) => Ok(None),
        },
        Err(err) => {
            Err(ProviderError::Unexpected(format!(
                "arXiv XML response had an unexpected format! Response body:\n{body}\nError message:\n{err}"
            )))
        }
    }
}
