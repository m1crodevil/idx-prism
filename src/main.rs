use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::Serialize;
use std::env;
use std::error::Error;
use std::fs;
use std::io::{BufRead, Read};
use std::path::PathBuf;
use std::process;

#[derive(Debug, Default, Serialize)]
struct InvestmentPropertyData {
    ticker: String,
    year: u32,
    current_year_instant: Option<i64>,
    prior_year_instant: Option<i64>,
    policy_text: String,
    accounting_model: String,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args = parse_args();

    let data = fs::read(&args.file)?;
    let (current_year_instant, prior_year_instant, policy_text) =
        extract_investment_properties(&data)?;
    let accounting_model = detect_model(&policy_text);

    let result = InvestmentPropertyData {
        ticker: args.ticker.to_uppercase(),
        year: args.year,
        current_year_instant,
        prior_year_instant,
        policy_text,
        accounting_model,
    };

    let json = serde_json::to_string_pretty(&result)?;
    if let Some(out) = args.output {
        fs::write(&out, json)?;
        println!("Saved to {}", out.display());
    } else {
        println!("{}", json);
    }

    Ok(())
}

struct Args {
    ticker: String,
    year: u32,
    file: PathBuf,
    output: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut ticker = String::new();
    let mut year = 2024u32;
    let mut file = PathBuf::new();
    let mut output: Option<PathBuf> = None;

    let mut iter = env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-y" | "--year" => {
                if let Some(v) = iter.next() {
                    year = v.parse().unwrap_or(2024);
                }
            }
            "-f" | "--file" => {
                if let Some(v) = iter.next() {
                    file = PathBuf::from(v);
                }
            }
            "-o" | "--output" => {
                if let Some(v) = iter.next() {
                    output = Some(PathBuf::from(v));
                }
            }
            s if s.starts_with('-') => {}
            s => ticker = s.to_string(),
        }
    }

    if ticker.is_empty() || file.as_os_str().is_empty() {
        eprintln!("usage: idxlens_rust <ticker> -f <instance.zip> [-y <year>] [-o <output.json>]");
        process::exit(1);
    }

    Args {
        ticker,
        year,
        file,
        output,
    }
}

fn extract_investment_properties(
    data: &[u8],
) -> Result<(Option<i64>, Option<i64>, String), Box<dyn Error>> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data))?;

    let mut xbrl_bytes = None;
    for i in 0..zip.len() {
        let file = zip.by_index(i)?;
        if file.name().ends_with(".xbrl") || file.name().ends_with(".xml") {
            xbrl_bytes = Some(read_zip_entry(file)?);
            break;
        }
    }

    let xbrl = xbrl_bytes.ok_or("no XBRL file found in archive")?;
    parse_investment_properties(&xbrl)
}

fn read_zip_entry(mut file: zip::read::ZipFile) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(buf)
}

fn parse_investment_properties(
    xml: &[u8],
) -> Result<(Option<i64>, Option<i64>, String), Box<dyn Error>> {
    let (current, prior) = parse_numeric_facts(xml)?;
    let policy = parse_policy_text(xml).unwrap_or_default();
    Ok((current, prior, policy))
}

fn parse_numeric_facts(
    xml: &[u8],
) -> Result<(Option<i64>, Option<i64>), Box<dyn Error>> {
    let mut reader = Reader::from_reader(xml);

    let mut current: Option<i64> = None;
    let mut prior: Option<i64> = None;

    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                let local = local_name(&reader, &e)?;
                if local == "InvestmentProperties" {
                    if let Some(ctx) = get_attr(&e, b"contextRef") {
                        let value_text = read_text_content(&mut reader)?;
                        let value = value_text.replace(',', "").parse::<i64>().ok();
                        match ctx.as_str() {
                            "CurrentYearInstant" => current = value,
                            "PriorEndYearInstant" => prior = value,
                            _ => {}
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok((current, prior))
}

fn parse_policy_text(xml: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(xml);

    // idx-cor is the observed namespace prefix for IDX taxonomy text blocks.
    // Try the plural form first; fall back to the singular form if present.
    if let Some(m) = regex::Regex::new(
        r"(?s)<([\w-]+:)?InvestmentPropertiesTextBlock[^>]*>(.*?)</([\w-]+:)?InvestmentPropertiesTextBlock>"
    )
    .ok()?
    .captures(&text)
    {
        let raw = m.get(2)?.as_str();
        if !raw.trim().is_empty() {
            return Some(clean_policy_text(raw));
        }
    }

    if let Some(m) = regex::Regex::new(
        r"(?s)<([\w-]+:)?InvestmentPropertyTextBlock[^>]*>(.*?)</([\w-]+:)?InvestmentPropertyTextBlock>"
    )
    .ok()?
    .captures(&text)
    {
        let raw = m.get(2)?.as_str();
        if !raw.trim().is_empty() {
            return Some(clean_policy_text(raw));
        }
    }

    None
}

fn clean_policy_text(raw: &str) -> String {
    raw.split(|c: char| c == '<' || c == '>')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn local_name(reader: &Reader<&[u8]>, e: &BytesStart<'_>) -> Result<String, Box<dyn Error>> {
    Ok(reader
        .decoder()
        .decode(e.name().local_name().as_ref())?
        .to_string())
}

fn get_attr(e: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    e.try_get_attribute(name)
        .ok()
        .flatten()
        .and_then(|a| String::from_utf8(a.value.into_owned()).ok())
}

fn read_text_content<R: BufRead>(reader: &mut Reader<R>) -> Result<String, Box<dyn Error>> {
    let mut buf = Vec::new();
    let text = reader.read_event_into(&mut buf)?;
    match text {
        Event::Text(t) => Ok(t.unescape()?.to_string()),
        _ => Ok(String::new()),
    }
}

fn detect_model(policy: &str) -> String {
    let lower = policy.to_lowercase();
    if lower.contains("model nilai wajar") {
        "fair value model".to_string()
    } else if lower.contains("biaya") {
        "cost model".to_string()
    } else {
        "unknown".to_string()
    }
}
