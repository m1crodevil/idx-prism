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
struct FinancialReportData {
    ticker: String,
    year: u32,
    accounting_model: String,
    policy_text: String,

    // Properti Investasi (carrying amounts)
    current_year_instant: Option<i64>,
    prior_year_instant: Option<i64>,

    // Variabel Kontrol Karya Akhir (XBRL)
    #[serde(skip_serializing_if = "Option::is_none")]
    total_assets: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_liabilities: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    equity: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revenues: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    net_income: Option<i64>,

    // Variabel Pengungkapan Nilai Wajar (PDF CALK)
    #[serde(skip_serializing_if = "Option::is_none")]
    pdf_fair_value_amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pdf_appraiser_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pdf_appraisal_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pdf_property_location_composition: Option<String>,
}

#[derive(Default)]
struct XbrlNumericFacts {
    current_year_instant: Option<i64>,
    prior_year_instant: Option<i64>,
    total_assets: Option<i64>,
    total_liabilities: Option<i64>,
    equity: Option<i64>,
    revenues: Option<i64>,
    net_income: Option<i64>,
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
    let (facts, mut policy_text) = extract_xbrl_data(&data)?;
    let mut accounting_model = detect_model(&policy_text);

    let mut pdf_fair_value_amount = None;
    let mut pdf_appraiser_name = None;
    let mut pdf_appraisal_date = None;
    let mut pdf_property_location_composition = None;

    if let Some(pdf_path) = &args.pdf {
        let extracted = pdf_extract::extract_pdf_fields(pdf_path)?;
        pdf_fair_value_amount = extracted.fair_value_amount;
        pdf_appraiser_name = extracted.appraiser_name;
        pdf_appraisal_date = extracted.appraisal_date;
        pdf_property_location_composition = extracted.property_location_composition;

        // Fallback: jika policy_text di XBRL hanya pointer generik / kosong
        if policy_text.trim().to_lowercase().starts_with("idem row")
            || policy_text.trim().len() < 20
        {
            if let Some(pdf_policy) = extracted.accounting_policy {
                policy_text = pdf_policy;
                accounting_model = detect_model(&policy_text);
            }
        }
    }

    let result = FinancialReportData {
        ticker: args.ticker.to_uppercase(),
        year: args.year,
        accounting_model,
        policy_text,
        current_year_instant: facts.current_year_instant,
        prior_year_instant: facts.prior_year_instant,
        total_assets: facts.total_assets,
        total_liabilities: facts.total_liabilities,
        equity: facts.equity,
        revenues: facts.revenues,
        net_income: facts.net_income,
        pdf_fair_value_amount,
        pdf_appraiser_name,
        pdf_appraisal_date,
        pdf_property_location_composition,
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
    let mut year: Option<u32> = None;
    let mut file = PathBuf::new();
    let mut pdf: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;

    let mut iter = env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-y" | "--year" => {
                if let Some(v) = iter.next() {
                    year = v.parse().ok();
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

    let year = match year {
        Some(y) => y,
        None => {
            eprintln!("Error: -y / --year is required (e.g. -y 2023)");
            process::exit(1);
        }
    };

    if ticker.is_empty() || file.as_os_str().is_empty() {
        eprintln!("usage: idxlens_rust <ticker> -f <instance.zip> -y <year> [--pdf <annual_report.pdf>] [-o <output.json>]");
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

fn extract_xbrl_data(data: &[u8]) -> Result<(XbrlNumericFacts, String), Box<dyn Error>> {
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
    let facts = parse_numeric_facts(&xbrl)?;
    let policy = parse_policy_text(&xbrl).unwrap_or_default();
    Ok((facts, policy))
}

fn read_zip_entry(mut file: zip::read::ZipFile) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(buf)
}

fn parse_numeric_facts(xml: &[u8]) -> Result<XbrlNumericFacts, Box<dyn Error>> {
    let mut reader = Reader::from_reader(xml);
    let mut facts = XbrlNumericFacts::default();

    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                let local = local_name(&reader, &e)?;
                if let Some(ctx) = get_attr(&e, b"contextRef") {
                    let is_current_instant =
                        ctx == "CurrentYearInstant" || ctx == "CurrentPeriodInstant";
                    let is_current_duration =
                        ctx == "CurrentYearDuration" || ctx == "CurrentPeriodDuration";

                    match local.as_str() {
                        "InvestmentProperties" => {
                            let val = read_text_num(&mut reader)?;
                            if ctx == "CurrentYearInstant" {
                                facts.current_year_instant = val;
                            } else if ctx == "PriorEndYearInstant" {
                                facts.prior_year_instant = val;
                            }
                        }
                        "Assets" if is_current_instant => {
                            facts.total_assets = read_text_num(&mut reader)?;
                        }
                        "Liabilities" if is_current_instant => {
                            facts.total_liabilities = read_text_num(&mut reader)?;
                        }
                        "Equity" if is_current_instant => {
                            facts.equity = read_text_num(&mut reader)?;
                        }
                        "SalesAndRevenue" | "Revenues" if is_current_duration => {
                            // Ambil hanya jika belum terisi atau konsolidasi agregat
                            if facts.revenues.is_none() {
                                facts.revenues = read_text_num(&mut reader)?;
                            }
                        }
                        "ProfitLoss" | "NetIncomeLoss" if is_current_duration => {
                            if facts.net_income.is_none() {
                                facts.net_income = read_text_num(&mut reader)?;
                            }
                        }
                        _ => {}
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok(facts)
}

fn read_text_num<R: BufRead>(reader: &mut Reader<R>) -> Result<Option<i64>, Box<dyn Error>> {
    let text = read_text_content(reader)?;
    Ok(text.replace(',', "").trim().parse::<i64>().ok())
}

fn parse_policy_text(xml: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(xml);

    // ponytail: fallback regex karena quick-xml miss tag plural idx-cor:InvestmentPropertiesTextBlock
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
    let mut in_tag = false;
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
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
    use std::error::Error;
    use std::path::Path;

    #[derive(Debug, Default)]
    pub struct PdfExtracted {
        pub fair_value_amount: Option<String>,
        pub appraiser_name: Option<String>,
        pub appraisal_date: Option<String>,
        pub property_location_composition: Option<String>,
        pub accounting_policy: Option<String>,
    }

    pub fn extract_pdf_fields<P: AsRef<Path>>(path: P) -> Result<PdfExtracted, Box<dyn Error>> {
        let text = extract_pdf_text(path)?;
        let lower = text.to_lowercase();

        Ok(PdfExtracted {
            fair_value_amount: extract_fair_value_amount(&text, &lower),
            appraiser_name: extract_appraiser_name(&text, &lower),
            appraisal_date: extract_appraisal_date(&text, &lower),
            property_location_composition: extract_property_location(&text, &lower),
            accounting_policy: extract_accounting_policy(&text, &lower),
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

    fn extract_fair_value_amount(text: &str, lower: &str) -> Option<String> {
        if !lower.contains("nilai wajar") && !lower.contains("fair value") {
            return None;
        }

        let re = Regex::new(
            r"(?i)(nilai wajar properti investasi[\s\S]{0,80}sebesar\s*(?:Rp\.?\s?)?[\d.,]+[\s\S]{0,300})\.\s"
        ).ok()?;
        if let Some(m) = re.find(text) {
            return Some(m.as_str().trim().to_string());
        }

        let re = Regex::new(
            r"(?i)(fair values of certain investment properties[\s\S]{0,100}(?:amounted to|sebesar)[\s\S]{0,80}(?:Rp\.?\s?)?[\d.,]+[\s\S]{0,300})\.\s"
        ).ok()?;
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

    fn extract_accounting_policy(text: &str, _lower: &str) -> Option<String> {
        let re = Regex::new(
            r"(?i)(?:properti investasi adalah|investment properties are)[\s\S]{0,800}?(?:penurunan nilai|impairment)"
        ).ok()?;
        if let Some(m) = re.find(text) {
            return Some(m.as_str().trim().to_string());
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_model_and_clean_text() {
        assert_eq!(
            detect_model("Perusahaan menggunakan model nilai wajar"),
            "fair value model"
        );
        assert_eq!(
            detect_model("measured at cost less accumulated depreciation"),
            "cost model"
        );
        assert_eq!(detect_model("Idem row 10"), "unknown");
        assert_eq!(
            clean_policy_text("<p>Biaya <b>perolehan</b></p>"),
            "Biaya perolehan"
        );
    }
}
