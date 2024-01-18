use chrono::{DateTime, Datelike, Local};
use itertools::Itertools;
use regex::Regex;
use reqwest::StatusCode;
use serde::Deserialize;

use crate::entry::{Entry, Fields};
use crate::source::RecordError;

pub const IDENTIFIER_REGEX: &'static str = concat!(
    r"^(",
    // old style:
    r"(",
    // first, the archive
    r"(acc-phys|adap-org|alg-geom|ao-sci|astro-ph|astro-ph\.CO|astro-ph\.EP|astro-ph\.GA|astro-ph\.HE|astro-ph\.IM|astro-ph\.SR|atom-ph|bayes-an|chao-dyn|chem-ph|cmp-lg|comp-gas|cond-mat|cond-mat\.dis-nn|cond-mat\.mes-hall|cond-mat\.mtrl-sci|cond-mat\.other|cond-mat\.quant-gas|cond-mat\.soft|cond-mat\.stat-mech|cond-mat\.str-el|cond-mat\.supr-con|cs\.AI|cs\.AR|cs\.CC|cs\.CE|cs\.CG|cs\.CL|cs\.CR|cs\.CV|cs\.CY|cs\.DB|cs\.DC|cs\.DL|cs\.DM|cs\.DS|cs\.ET|cs\.FL|cs\.GL|cs\.GR|cs\.GT|cs\.HC|cs\.IR|cs\.IT|cs\.LG|cs\.LO|cs\.MA|cs\.MM|cs\.MS|cs\.NA|cs\.NE|cs\.NI|cs\.OH|cs\.OS|cs\.PF|cs\.PL|cs\.RO|cs\.SC|cs\.SD|cs\.SE|cs\.SI|cs\.SY|dg-ga|econ\.EM|econ\.GN|econ\.TH|eess\.AS|eess\.IV|eess\.SP|eess\.SY|funct-an|gr-qc|hep-ex|hep-lat|hep-ph|hep-th|math-ph|math\.AC|math\.AG|math\.AP|math\.AT|math\.CA|math\.CO|math\.CT|math\.CV|math\.DG|math\.DS|math\.FA|math\.GM|math\.GN|math\.GR|math\.GT|math\.HO|math\.IT|math\.KT|math\.LO|math\.MG|math\.MP|math\.NA|math\.NT|math\.OA|math\.OC|math\.PR|math\.QA|math\.RA|math\.RT|math\.SG|math\.SP|math\.ST|mtrl-th|nlin\.AO|nlin\.CD|nlin\.CG|nlin\.PS|nlin\.SI|nucl-ex|nucl-th|patt-sol|physics\.acc-ph|physics\.ao-ph|physics\.app-ph|physics\.atm-clus|physics\.atom-ph|physics\.bio-ph|physics\.chem-ph|physics\.class-ph|physics\.comp-ph|physics\.data-an|physics\.ed-ph|physics\.flu-dyn|physics\.gen-ph|physics\.geo-ph|physics\.hist-ph|physics\.ins-det|physics\.med-ph|physics\.optics|physics\.plasm-ph|physics\.pop-ph|physics\.soc-ph|physics\.space-ph|plasm-ph|q-alg|q-bio|q-bio\.BM|q-bio\.CB|q-bio\.GN|q-bio\.MN|q-bio\.NC|q-bio\.OT|q-bio\.PE|q-bio\.QM|q-bio\.SC|q-bio\.TO|q-fin\.CP|q-fin\.EC|q-fin\.GN|q-fin\.MF|q-fin\.PM|q-fin\.PR|q-fin\.RM|q-fin\.ST|q-fin\.TR|quant-ph|solv-int|stat\.AP|stat\.CO|stat\.ME|stat\.ML|stat\.OT|stat\.TH|supr-con)",
    r"/",
    // followed by the identifier
    r"(([0-1][0-9])|(9[1-9]))(0[1-9]|1[0-2])([0-9]{3})(v[1-9][0-9]*)?",
    r")",
    r"|",
    // new style: YYMM.NNNN or YYMM.NNNNN or with version
    r"([0-9][0-9](0[1-9]|1[0-2])[.][0-9]{4,5})(v[1-9][0-9]*)?",
    r")$",
);

#[derive(Deserialize, Debug)]
struct ArxivXML {
    entry: Vec<ArxivXMLEntry>,
}

#[derive(Deserialize, Debug)]
struct ArxivXMLEntry {
    title: String,
    author: Vec<ArxivXMLAuthor>,
    id: String,
    updated: DateTime<Local>,
    published: DateTime<Local>,
}

#[derive(Deserialize, Debug)]
struct ArxivXMLAuthor {
    name: String,
}

impl From<ArxivXMLEntry> for Entry {
    fn from(arxiv_xml: ArxivXMLEntry) -> Entry {
        Entry {
            entry_type: "preprint".to_string(),
            fields: Fields {
                title: Some(arxiv_xml.title),
                author: Some(
                    arxiv_xml
                        .author
                        .into_iter()
                        .map(|auth| auth.name)
                        .join(" and "),
                ),
                year: Some(arxiv_xml.updated.year().to_string()),
                ..Fields::default()
            },
        }
    }
}

pub fn get_record(id: &str) -> Result<Option<Entry>, RecordError> {
    let response = reqwest::blocking::get(format!(
        "https://export.arxiv.org/api/query?max_results=250&id_list={id}"
    ))?;

    let body = match response.status() {
        StatusCode::OK => response.text()?,
        StatusCode::NOT_FOUND => {
            return Ok(None);
        }
        code => return Err(RecordError::UnexpectedStatusCode(code)),
    };

    match quick_xml::de::from_str::<ArxivXML>(&body) {
        Ok(parsed) => {
            let first_entry = parsed.entry.into_iter().nth(0).unwrap();
            Ok(Some(first_entry.into()))
        }
        Err(_) => Err(RecordError::UnexpectedFailure(
            "arxiv xml response has unexpected format!".to_string(),
        )),
    }
}

pub fn is_valid_id(id: &str) -> bool {
    let arxiv_identifier_regex = Regex::new(IDENTIFIER_REGEX).unwrap();
    arxiv_identifier_regex.is_match(id)
}
