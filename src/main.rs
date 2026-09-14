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

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let args = parse_args();

    let data = fs::read(&args.file)?;
    let mut report = extract_xbrl_data(&data)?;
    report.ticker = args.ticker.to_uppercase();
    report.year = args.year;
    report.accounting_model = detect_model(&report.policy_text);

    if let Some(pdf_path) = &args.pdf {
        let extracted = pdf_extract::extract_pdf_fields(pdf_path)?;
        report.pdf_fair_value_amount = extracted.fair_value_amount;
        report.pdf_appraiser_name = extracted.appraiser_name;
        report.pdf_appraisal_date = extracted.appraisal_date;
        report.pdf_property_location_composition = extracted.property_location_composition;

        // Fallback: jika policy_text di XBRL berupa pointer ("Idem row"), kosong,
        // atau tidak menghasilkan klasifikasi model yang jelas (unknown)
        if report.accounting_model == "unknown"
            || report
                .policy_text
                .trim()
                .to_lowercase()
                .starts_with("idem row")
            || report.policy_text.trim().len() < 20
        {
            if let Some(ref pdf_policy) = extracted.accounting_policy {
                let model_from_pdf = detect_model(pdf_policy);
                if model_from_pdf != "unknown" {
                    report.accounting_model = model_from_pdf;
                    if report.policy_text.trim().len() < 20
                        || report
                            .policy_text
                            .trim()
                            .to_lowercase()
                            .starts_with("idem row")
                    {
                        report.policy_text = pdf_policy.clone();
                    }
                }
            }
        }
    }

    let json = serde_json::to_string_pretty(&report)?;
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

fn extract_xbrl_data(data: &[u8]) -> Result<FinancialReportData, Box<dyn Error>> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(data))?;

    let mut xbrl_bytes = None;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        if file.name().ends_with(".xbrl") || file.name().ends_with(".xml") {
            let mut buf = Vec::with_capacity(file.size() as usize);
            file.read_to_end(&mut buf)?;
            xbrl_bytes = Some(buf);
            break;
        }
    }

    let xbrl = xbrl_bytes.ok_or("no XBRL file found in archive")?;
    let mut report = FinancialReportData::default();
    parse_numeric_facts(&xbrl, &mut report)?;
    report.policy_text = parse_policy_text(&xbrl).unwrap_or_default();
    Ok(report)
}

fn parse_numeric_facts(xml: &[u8], report: &mut FinancialReportData) -> Result<(), Box<dyn Error>> {
    let mut reader = Reader::from_reader(xml);

    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                if let Some(ctx) = get_attr(&e, b"contextRef") {
                    let is_current_instant =
                        ctx == "CurrentYearInstant" || ctx == "CurrentPeriodInstant";
                    let is_current_duration =
                        ctx == "CurrentYearDuration" || ctx == "CurrentPeriodDuration";

                    match e.name().local_name().as_ref() {
                        b"InvestmentProperties" => {
                            let val = read_text_num(&mut reader)?;
                            if ctx == "CurrentYearInstant" {
                                report.current_year_instant = val;
                            } else if ctx == "PriorEndYearInstant" {
                                report.prior_year_instant = val;
                            }
                        }
                        b"Assets" if is_current_instant => {
                            report.total_assets = read_text_num(&mut reader)?;
                        }
                        b"Liabilities" if is_current_instant => {
                            report.total_liabilities = read_text_num(&mut reader)?;
                        }
                        b"Equity" if is_current_instant => {
                            report.equity = read_text_num(&mut reader)?;
                        }
                        b"SalesAndRevenue" | b"Revenues" if is_current_duration => {
                            if report.revenues.is_none() {
                                report.revenues = read_text_num(&mut reader)?;
                            }
                        }
                        b"ProfitLoss" | b"NetIncomeLoss" if is_current_duration => {
                            if report.net_income.is_none() {
                                report.net_income = read_text_num(&mut reader)?;
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

    Ok(())
}

fn read_text_num<R: BufRead>(reader: &mut Reader<R>) -> Result<Option<i64>, Box<dyn Error>> {
    let text = read_text_content(reader)?;
    Ok(text.replace(',', "").trim().parse::<i64>().ok())
}

fn parse_policy_text(xml: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(xml);

    // ponytail: fallback regex karena quick-xml miss tag plural idx-cor:InvestmentPropertiesTextBlock
    let plural = Regex::new(
        r"(?s)<([\w-]+:)?InvestmentPropertiesTextBlock[^>]*>(.*?)</([\w-]+:)?InvestmentPropertiesTextBlock>",
    )
    .ok()?;
    if let Some(m) = plural.captures(&text) {
        let raw = m.get(2)?.as_str();
        if !raw.trim().is_empty() {
            return Some(clean_policy_text(raw));
        }
    }

    let singular = Regex::new(
        r"(?s)<([\w-]+:)?InvestmentPropertyTextBlock[^>]*>(.*?)</([\w-]+:)?InvestmentPropertyTextBlock>",
    )
    .ok()?;
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
    } else if lower.contains("biaya")
        || lower.contains("cost")
        || lower.contains("nilai perolehan")
        || lower.contains("akumulasi penyusutan")
        || lower.contains("accumulated depreciation")
    {
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
        let ip_region = investment_property_region(&text, &lower);

        Ok(PdfExtracted {
            fair_value_amount: extract_fair_value_amount(ip_region),
            appraiser_name: extract_appraiser_name(ip_region),
            appraisal_date: extract_appraisal_date(ip_region),
            property_location_composition: extract_property_location(ip_region),
            accounting_policy: extract_accounting_policy(&text),
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

    fn investment_property_region<'a>(text: &'a str, lower: &'a str) -> &'a str {
        let markers = [
            "nilai wajar properti investasi",
            "fair values of certain investment properties",
            "16. properti investasi",
            "16. investment properties",
            "14. investment properties",
            "14. properti investasi",
            "13. investment properties",
            "13. properti investasi",
            "10. properti investasi",
            "10. investment properties",
            "properti investasi - neto",
            "properti investasi",
        ];
        for marker in markers {
            if let Some(pos) = lower.find(marker) {
                let end = text.len().min(pos + 35000);
                return &text[pos..end];
            }
        }
        text
    }

    fn extract_fair_value_amount(text: &str) -> Option<String> {
        let lower = text.to_lowercase();
        if !lower.contains("nilai wajar") && !lower.contains("fair value") {
            return None;
        }

        let re = Regex::new(
            r"(?i)(nilai wajar[\s\S]{0,120}?properti investasi[\s\S]{0,150}?(?:sebesar|amounted to)\s*(?:Rp\.?\s*)?[\d.,]+[^\n.]{0,80}?(?:ribu|jutaan)?[\s\S]{0,350}?\.\s)",
        )
        .ok()?;
        if let Some(m) = re.find(text) {
            return Some(m.as_str().trim().to_string());
        }

        let re = Regex::new(
            r"(?i)(fair values of certain investment properties[\s\S]{0,100}(?:amounted to|sebesar)[\s\S]{0,80}(?:Rp\.?\s?)?[\d.,]+[\s\S]{0,300})\.\s",
        )
        .ok()?;
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

    fn extract_appraiser_name(text: &str) -> Option<String> {
        // Pattern 1: Temukan KJPP langsung dalam region catatan properti investasi
        let re_kjpp = Regex::new(
            r"(?i)(KJPP\s+[A-Za-z0-9\s&,–-]{3,60}?(?:Rekan|\(Rengganis\)|\(Putri\)|dan Rekan|& Rekan))",
        )
        .ok()?;
        let mut appraisers = Vec::new();
        for caps in re_kjpp.captures_iter(text) {
            if let Some(m) = caps.get(1) {
                let name = m.as_str().replace('\n', " ").trim().to_string();
                if !appraisers.contains(&name) {
                    appraisers.push(name);
                }
            }
        }
        if !appraisers.is_empty() {
            return Some(appraisers.join(", "));
        }

        // Pattern 2: Fallback kalimat umum penilai independen
        let re = Regex::new(
            r"(?i)(?:nilai wajar properti investasi|fair values of certain investment properties)[\s\S]{0,180}((?:penilai independen|independent appraisers)[\s\S]{0,350}?)\.\s",
        )
        .ok()?;
        if let Some(caps) = re.captures(text) {
            if let Some(m) = caps.get(1) {
                return Some(m.as_str().trim().to_string());
            }
        }

        None
    }

    fn extract_appraisal_date(text: &str) -> Option<String> {
        let re = Regex::new(
            r"(?i)(?:laporan terakhir tanggal|latest report dated|date of report|tanggal laporan|tertanggal)[^\d]{0,30}(\d{1,2}\s+[A-Za-z/]+\s+\d{4})",
        )
        .ok()?;
        re.find(text).map(|m| m.as_str().trim().to_string())
    }

    fn extract_property_location(text: &str) -> Option<String> {
        let re = Regex::new(
            r"(?i)(?:properti investasi (?:terutama )?merupakan|investment properties (?:mainly )?represent)[\s\S]{0,500}?\.\s",
        )
        .ok()?;
        if let Some(m) = re.find(text) {
            return Some(m.as_str().trim().to_string());
        }

        let re = Regex::new(
            r"(?i)(?:properti investasi|investment properties)[\s\S]{0,200}(?:terletak di|located in)[\s\S]{0,80}?\.\s",
        )
        .ok()?;
        re.find(text).map(|m| m.as_str().trim().to_string())
    }

    fn extract_accounting_policy(text: &str) -> Option<String> {
        let re = Regex::new(
            r"(?i)(?:properti investasi (?:adalah|terdiri dari|diukur|dinyatakan)|investment properties (?:are|consisting of|measured|stated))[\s\S]{0,800}?(?:penurunan nilai|penyusutan|impairment|depreciation)",
        )
        .ok()?;
        if let Some(m) = re.find(text) {
            return Some(m.as_str().trim().to_string());
        }

        let re = Regex::new(
            r"(?i)(?:penyusutan dihitung|depreciation is computed)[^.]{0,100}(?:garis lurus|straightline|useful life|masa manfaat)[^.]{0,200}",
        )
        .ok()?;
        if let Some(m) = re.find(text) {
            return Some(m.as_str().trim().to_string());
        }

        None
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
        assert_eq!(
            detect_model("diukur sebesar nilai perolehan setelah dikurangi akumulasi penyusutan"),
            "cost model"
        );
        assert_eq!(detect_model("Idem row 10"), "unknown");
        assert_eq!(
            clean_policy_text("<p>Biaya <b>perolehan</b></p>"),
            "Biaya perolehan"
        );
    }
}
