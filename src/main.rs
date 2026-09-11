use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use regex::Regex;
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

    // PDF-derived fields
    #[serde(skip_serializing_if = "Option::is_none")]
    pdf_fair_value_amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pdf_appraiser_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pdf_appraisal_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pdf_valuation_method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pdf_valuation_assumptions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pdf_property_location_composition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pdf_depreciation_policy: Option<String>,
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
    let (current_year_instant, prior_year_instant, mut policy_text) =
        extract_investment_properties(&data)?;
    let mut accounting_model = detect_model(&policy_text);

    let mut pdf_fair_value_amount = None;
    let mut pdf_appraiser_name = None;
    let mut pdf_appraisal_date = None;
    let mut pdf_valuation_method = None;
    let mut pdf_valuation_assumptions = None;
    let mut pdf_property_location_composition = None;
    let mut pdf_depreciation_policy = None;

    if let Some(pdf_path) = &args.pdf {
        let extracted = pdf_extract::extract_pdf_fields(pdf_path)?;
        pdf_fair_value_amount = extracted.fair_value_amount;
        pdf_appraiser_name = extracted.appraiser_name;
        pdf_appraisal_date = extracted.appraisal_date;
        pdf_valuation_method = extracted.valuation_method;
        pdf_valuation_assumptions = extracted.valuation_assumptions;
        pdf_property_location_composition = extracted.property_location_composition;
        pdf_depreciation_policy = extracted.depreciation_policy;

        // Fallback: if XBRL policy text is unusable, derive from PDF accounting policy note.
        if policy_text.trim().to_lowercase().starts_with("idem row")
            || policy_text.trim().len() < 20
        {
            if let Some(ref pdf_policy) = pdf_depreciation_policy {
                policy_text = pdf_policy.clone();
                accounting_model = detect_model(&policy_text);
            }
        }
    }

    let result = InvestmentPropertyData {
        ticker: args.ticker.to_uppercase(),
        year: args.year,
        current_year_instant,
        prior_year_instant,
        policy_text,
        accounting_model,
        pdf_fair_value_amount,
        pdf_appraiser_name,
        pdf_appraisal_date,
        pdf_valuation_method,
        pdf_valuation_assumptions,
        pdf_property_location_composition,
        pdf_depreciation_policy,
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
    pdf: Option<PathBuf>,
    output: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut ticker = String::new();
    let mut year = 2024u32;
    let mut file = PathBuf::new();
    let mut pdf: Option<PathBuf> = None;
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
            "--pdf" => {
                if let Some(v) = iter.next() {
                    pdf = Some(PathBuf::from(v));
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
        eprintln!("usage: idxlens_rust <ticker> -f <instance.zip> [--pdf <annual_report.pdf>] [-y <year>] [-o <output.json>]");
        process::exit(1);
    }

    Args {
        ticker,
        year,
        file,
        pdf,
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

fn parse_numeric_facts(xml: &[u8]) -> Result<(Option<i64>, Option<i64>), Box<dyn Error>> {
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

    let plural = Regex::new(
        r"(?s)<([\w-]+:)?InvestmentPropertiesTextBlock[^>]*>(.*?)</([\w-]+:)?InvestmentPropertiesTextBlock>"
    ).ok()?;
    if let Some(m) = plural.captures(&text) {
        let raw = m.get(2)?.as_str();
        if !raw.trim().is_empty() {
            return Some(clean_policy_text(raw));
        }
    }

    let singular = Regex::new(
        r"(?s)<([\w-]+:)?InvestmentPropertyTextBlock[^>]*>(.*?)</([\w-]+:)?InvestmentPropertyTextBlock>"
    ).ok()?;
    if let Some(m) = singular.captures(&text) {
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
    if lower.contains("model nilai wajar") || lower.contains("fair value model") {
        "fair value model".to_string()
    } else if lower.contains("biaya") || lower.contains("cost") {
        "cost model".to_string()
    } else {
        "unknown".to_string()
    }
}

mod pdf_extract {
    use regex::Regex;
    use serde::Serialize;
    use std::error::Error;
    use std::path::Path;

    #[derive(Debug, Default, Serialize)]
    pub struct PdfExtracted {
        pub fair_value_amount: Option<String>,
        pub appraiser_name: Option<String>,
        pub appraisal_date: Option<String>,
        pub valuation_method: Option<String>,
        pub valuation_assumptions: Option<String>,
        pub property_location_composition: Option<String>,
        pub depreciation_policy: Option<String>,
    }

    pub fn extract_pdf_fields<P: AsRef<Path>>(path: P) -> Result<PdfExtracted, Box<dyn Error>> {
        let text = extract_pdf_text(path)?;
        let lower = text.to_lowercase();

        let ip_region = investment_property_region(&text, &lower);

        Ok(PdfExtracted {
            fair_value_amount: extract_fair_value_amount(&text, &lower),
            appraiser_name: extract_appraiser_name(&text, &lower),
            appraisal_date: extract_appraisal_date(&text, &lower),
            valuation_method: extract_valuation_method(&ip_region),
            valuation_assumptions: extract_valuation_assumptions(&ip_region),
            property_location_composition: extract_property_location(&text, &lower),
            depreciation_policy: extract_depreciation_policy(&text, &lower),
        })
    }

    fn extract_pdf_text<P: AsRef<Path>>(path: P) -> Result<String, Box<dyn Error>> {
        use pdf_oxide::PdfDocument;
        let doc = PdfDocument::open(path.as_ref())?;
        let mut text = String::new();
        let page_count = doc.page_count()?;
        for i in 0..page_count {
            if let Ok(t) = doc.extract_text(i) {
                text.push_str(&t);
                text.push('\n');
            }
        }
        Ok(text)
    }

    /// Return a window of text covering the investment property note, if we can find it.
    fn investment_property_region<'a>(text: &'a str, lower: &'a str) -> &'a str {
        let markers = [
            // Prefer in-note fair value sentence first; it anchors us inside the actual note.
            "nilai wajar properti investasi",
            "fair values of certain investment properties",
            "14. investment properties",
            "14. properti investasi",
            "13. investment properties",
            "13. properti investasi",
            "investment properties - net",
            "properti investasi - neto",
        ];
        for marker in markers {
            if let Some(pos) = lower.find(marker) {
                let end = text.len().min(pos + 8000);
                return &text[pos..end];
            }
        }
        text
    }

    fn extract_fair_value_amount(text: &str, lower: &str) -> Option<String> {
        if !lower.contains("nilai wajar") && !lower.contains("fair value") {
            return None;
        }

        // Indonesian: capture the sentence containing investment property fair value and the amount.
        // Stops at the first ". " sentence boundary after the amount.
        let re = Regex::new(
            r"(?i)(nilai wajar properti investasi[\s\S]{0,80}sebesar\s*(?:Rp\.?\s?)?[\d.,]+[\s\S]{0,300})\.\s"
        ).ok()?;
        if let Some(m) = re.find(text) {
            return Some(m.as_str().trim().to_string());
        }

        // English: "fair values of certain investment properties amounted to Rp..."
        let re = Regex::new(r"(?i)(fair values of certain investment properties[\s\S]{0,100}(?:amounted to|sebesar)[\s\S]{0,80}(?:Rp\.?\s?)?[\d.,]+[\s\S]{0,300})\.\s").ok()?;
        if let Some(m) = re.find(text) {
            return Some(m.as_str().trim().to_string());
        }

        let re = Regex::new(r"(?i)nilai wajar properti investasi[^.]{0,150}").ok()?;
        if let Some(m) = re.find(text) {
            return Some(m.as_str().trim().to_string());
        }

        let re = Regex::new(r"(?i)fair value of (?:the )?investment propert(?:y|ies)[^.,]{0,150}")
            .ok()?;
        re.find(text).map(|m| m.as_str().trim().to_string())
    }

    fn extract_appraiser_name(text: &str, _lower: &str) -> Option<String> {
        // Capture appraiser list appearing near the investment property fair value sentence.
        let re = Regex::new(
            r"(?i)(?:nilai wajar properti investasi|fair values of certain investment properties)[\s\S]{0,180}((?:penilai independen|independent appraisers)[\s\S]{0,350}?)\.\s"
        ).ok()?;
        if let Some(caps) = re.captures(text) {
            if let Some(m) = caps.get(1) {
                return Some(m.as_str().trim().to_string());
            }
        }

        let re = Regex::new(r"(?i)(?:penilai independen|independent appraisers)[\s\S]{0,350}?\.\s")
            .ok()?;
        re.find(text).map(|m| m.as_str().trim().to_string())
    }

    fn extract_appraisal_date(text: &str, _lower: &str) -> Option<String> {
        let re = Regex::new(
            r"(?i)(?:laporan terakhir tanggal|latest report dated|date of report)[^\d]{0,20}(?:\d{1,2}\s+[A-Za-z]+\s+\d{4}|\d{1,2}/\d{1,2}/\d{4})"
        ).ok()?;
        re.find(text).map(|m| m.as_str().trim().to_string())
    }

    /// Look for valuation method keywords only within the investment property note region.
    fn extract_valuation_method(ip_region: &str) -> Option<String> {
        let lower = ip_region.to_lowercase();
        if lower.contains("pendekatan pendapatan") || lower.contains("income approach") {
            return Some("income approach".to_string());
        }
        if lower.contains("pendekatan pasar") || lower.contains("market approach") {
            return Some("market approach".to_string());
        }
        if lower.contains("pendekatan biaya") || lower.contains("cost approach") {
            return Some("cost approach".to_string());
        }
        if lower.contains("discounted cash flow") || lower.contains("dcf") {
            return Some("discounted cash flow".to_string());
        }
        None
    }

    /// Look for valuation assumptions within the investment property note region.
    fn extract_valuation_assumptions(ip_region: &str) -> Option<String> {
        let re = Regex::new(
            r"(?i)(?:highest and best use|penggunaan tertinggi dan terbaik|in estimating the fair value|dalam mengestimasi nilai wajar)[\s\S]{0,250}?\.\s"
        ).ok()?;
        re.find(ip_region).map(|m| m.as_str().trim().to_string())
    }

    fn extract_property_location(text: &str, _lower: &str) -> Option<String> {
        let re = Regex::new(
            r"(?i)(?:properti investasi terutama merupakan|investment properties mainly represent)[\s\S]{0,250}(?:terletak di|located in)[\s\S]{0,80}?\.\s"
        ).ok()?;
        if let Some(m) = re.find(text) {
            return Some(m.as_str().trim().to_string());
        }

        let re = Regex::new(
            r"(?i)(?:properti investasi|investment properties)[\s\S]{0,200}(?:terletak di|located in)[\s\S]{0,80}?\.\s"
        ).ok()?;
        re.find(text).map(|m| m.as_str().trim().to_string())
    }

    fn extract_depreciation_policy(text: &str, _lower: &str) -> Option<String> {
        // Prefer the full investment property accounting policy paragraph.
        let re = Regex::new(
            r"(?i)(?:properti investasi adalah|investment properties are)[\s\S]{0,800}?(?:penurunan nilai|impairment)"
        ).ok()?;
        if let Some(m) = re.find(text) {
            return Some(m.as_str().trim().to_string());
        }

        // Fallback to explicit depreciation sentence.
        let re = Regex::new(
            r"(?i)(?:penyusutan dihitung|depreciation is computed)[^.]{0,100}(?:garis lurus|straightline|useful life|masa manfaat)[^.]{0,200}"
        ).ok()?;
        if let Some(m) = re.find(text) {
            return Some(m.as_str().trim().to_string());
        }

        let re = Regex::new(
            r"(?i)(?:properti investasi|investment propert)[^.]{0,200}(?:penyusutan|depreciation)[^.]{0,300}"
        ).ok()?;
        re.find(text).map(|m| m.as_str().trim().to_string())
    }
}
