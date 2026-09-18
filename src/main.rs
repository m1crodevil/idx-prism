use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fs;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
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

    // Komposisi kepemilikan saham (CALK note Modal Saham)
    #[serde(skip_serializing_if = "Option::is_none")]
    free_float_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    public_shares: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shares_outstanding: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct StockItem {
    #[serde(rename = "Code")]
    code: String,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "ListingDate")]
    listing_date: String,
    #[serde(rename = "Shares", default)]
    shares: Option<f64>,
    #[serde(rename = "ListingBoard", default)]
    listing_board: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StockDataResponse {
    #[serde(rename = "recordsTotal", default)]
    records_total: usize,
    data: Vec<StockItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MarketMetrics {
    ticker: String,
    year: u32,
    trading_days: usize,
    simple_spread: f64,
    corwin_schultz_spread: f64,
    amihud_illiquidity: f64,
    zero_return_days: f64,
    volatility: f64,
    turnover: Option<f64>,
    annual_volume: u64,
}

#[derive(Debug, Clone, Default)]
struct DailyBar {
    year: u32,
    high: f64,
    low: f64,
    close: f64,
    volume: u64,
}

fn main() {
    let raw: Vec<String> = env::args().collect();
    let args = &raw[1..];

    let result = match args.first().map(String::as_str) {
        Some("sector") => run_sector(&args[1..]),
        Some("market") => run_market(&args[1..]),
        _ => run_extract(args),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        process::exit(1);
    }
}

// -------------------------------------------------------------------------
// Shared CLI flag reader
// -------------------------------------------------------------------------

/// Read `-flag value` pairs. Flags listed in `valued` consume the next arg as
/// their value; any other `-flag` is recorded bare (presence-only, e.g. `-h`);
/// everything else is positional, in order. Unknown flags are ignored, matching
/// the lenient behavior of the original per-subcommand parsers.
fn parse_flags(args: &[String], valued: &[&str]) -> (HashMap<String, String>, Vec<String>) {
    let mut map = HashMap::new();
    let mut positional = Vec::new();
    let mut iter = args.iter();

    while let Some(arg) = iter.next() {
        if valued.contains(&arg.as_str()) {
            if let Some(v) = iter.next() {
                map.insert(arg.clone(), v.clone());
            }
        } else if arg.starts_with('-') {
            map.insert(arg.clone(), String::new());
        } else {
            positional.push(arg.clone());
        }
    }

    (map, positional)
}

/// First flag present among `names` (alias resolution, first alias wins).
fn flag<'a>(m: &'a HashMap<String, String>, names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|n| m.get(*n)).map(String::as_str)
}

// -------------------------------------------------------------------------
// Subcommand 1: Financial & CALK Extract
// -------------------------------------------------------------------------

fn run_extract(raw_args: &[String]) -> Result<(), Box<dyn Error>> {
    let args = parse_extract_args(raw_args);

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
        if let Some((public_shares, total_shares, free_float_pct)) = extracted.ownership {
            report.public_shares = Some(public_shares);
            report.shares_outstanding = Some(total_shares);
            report.free_float_pct = Some((free_float_pct * 100.0).round() / 100.0);
        }

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

struct ExtractArgs {
    ticker: String,
    year: u32,
    file: PathBuf,
    pdf: Option<PathBuf>,
    output: Option<PathBuf>,
}

fn parse_extract_args(raw_args: &[String]) -> ExtractArgs {
    let (m, positional) = parse_flags(
        raw_args,
        &["-y", "--year", "-f", "--file", "--pdf", "-o", "--output"],
    );

    let year = flag(&m, &["-y", "--year"]).and_then(|v| v.parse::<u32>().ok());
    let file = flag(&m, &["-f", "--file"]).map(PathBuf::from);
    let ticker = positional.last().cloned().unwrap_or_default();

    let Some(year) = year else {
        eprintln!("Error: -y / --year is required (e.g. -y 2023)");
        process::exit(1);
    };
    let Some(file) = file.filter(|f| !f.as_os_str().is_empty()) else {
        eprintln!("usage: idx-prism <ticker> -f <instance.zip> -y <year> [--pdf <annual_report.pdf>] [-o <output.json>]");
        process::exit(1);
    };
    if ticker.is_empty() {
        eprintln!("usage: idx-prism <ticker> -f <instance.zip> -y <year> [--pdf <annual_report.pdf>] [-o <output.json>]");
        process::exit(1);
    }

    ExtractArgs {
        ticker,
        year,
        file,
        pdf: flag(&m, &["--pdf"]).map(PathBuf::from),
        output: flag(&m, &["-o", "--output"]).map(PathBuf::from),
    }
}

// -------------------------------------------------------------------------
// Subcommand 2: Sector Listing & Details
// -------------------------------------------------------------------------

struct SectorArgs {
    input: Option<PathBuf>,
    max_listing_date: Option<String>,
    exclude_boards: Vec<String>,
    output: Option<PathBuf>,
    format: String,
}

fn parse_sector_args(args: &[String]) -> SectorArgs {
    let (m, _) = parse_flags(
        args,
        &[
            "-i",
            "--input",
            "--max-listing-date",
            "--max-date",
            "--exclude-board",
            "--exclude-boards",
            "--format",
            "-o",
            "--output",
        ],
    );

    if m.contains_key("-h") || m.contains_key("--help") {
        eprintln!("usage: idx-prism sector [-i <securities.json>] [--max-listing-date <YYYY-MM-DD>] [--exclude-board <Board1,Board2>] [--format list|csv|json] [-o <output_file>]");
        process::exit(0);
    }

    SectorArgs {
        input: flag(&m, &["-i", "--input"]).map(PathBuf::from),
        max_listing_date: flag(&m, &["--max-listing-date", "--max-date"]).map(str::to_string),
        exclude_boards: flag(&m, &["--exclude-board", "--exclude-boards"])
            .map(|v| {
                v.split(',')
                    .map(str::trim)
                    .filter(|b| !b.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        output: flag(&m, &["-o", "--output"]).map(PathBuf::from),
        format: flag(&m, &["--format"])
            .map(str::to_lowercase)
            .unwrap_or_else(|| "list".to_string()),
    }
}

fn run_sector(raw_args: &[String]) -> Result<(), Box<dyn Error>> {
    let args = parse_sector_args(raw_args);

    let input_path = match args.input {
        Some(ref p) => p.clone(),
        None => resolve_default_securities_path()?,
    };

    let bytes = fs::read(&input_path).map_err(|e| {
        format!(
            "failed to read securities JSON from {}: {}",
            input_path.display(),
            e
        )
    })?;

    let stock_resp: StockDataResponse = serde_json::from_slice(&bytes)?;
    let (filtered, excluded_by_date, excluded_by_board) = filter_securities(
        stock_resp.data,
        args.max_listing_date.as_deref(),
        &args.exclude_boards,
    );

    if args.max_listing_date.is_some() || !args.exclude_boards.is_empty() {
        eprintln!("[Purposive Sampling Report]");
        eprintln!("- Total Emiten Sektor: {}", stock_resp.records_total);
        if let Some(ref max_d) = args.max_listing_date {
            eprintln!(
                "- Eliminasi Listing IPO setelah {} : -{}",
                max_d, excluded_by_date
            );
        }
        if !args.exclude_boards.is_empty() {
            eprintln!(
                "- Eliminasi Papan {:?} : -{}",
                args.exclude_boards, excluded_by_board
            );
        }
        eprintln!("- Final Sampel Penelitian: {}", filtered.len());
    } else {
        eprintln!(
            "[Sektor Properties & Real Estate BEI] Total Emiten: {}",
            filtered.len()
        );
    }

    let out_str = match args.format.as_str() {
        "csv" => {
            let mut s = String::from("Code,Name,ListingDate,IPO_Year,ListingBoard,Shares\n");
            for item in &filtered {
                let date_clean = &item.listing_date[..item.listing_date.len().min(10)];
                let ipo_year = if date_clean.len() >= 4 {
                    &date_clean[..4]
                } else {
                    ""
                };
                s.push_str(&format!(
                    "{},\"{}\",{},{},\"{}\",{:.0}\n",
                    item.code,
                    item.name.replace('"', "\"\""),
                    date_clean,
                    ipo_year,
                    item.listing_board.as_deref().unwrap_or(""),
                    item.shares.unwrap_or(0.0)
                ));
            }
            s
        }
        "json" => serde_json::to_string_pretty(&filtered)?,
        _ => {
            let codes: Vec<&str> = filtered.iter().map(|x| x.code.as_str()).collect();
            codes.join(",")
        }
    };

    if let Some(ref out_path) = args.output {
        fs::write(out_path, &out_str)?;
        eprintln!("Output tersimpan di {}", out_path.display());
    } else {
        println!("{}", out_str);
    }

    Ok(())
}

fn resolve_default_securities_path() -> Result<PathBuf, Box<dyn Error>> {
    let home = env::var("HOME").map_err(|_| "HOME environment variable not set")?;
    let p = PathBuf::from(home).join(".idxlens/data/idx_properties_securities.json");
    if !p.exists() {
        return Err(format!(
            "File data emiten tidak ditemukan di {}. Gunakan -i <path/to/securities.json>.",
            p.display()
        )
        .into());
    }
    Ok(p)
}

fn filter_securities(
    items: Vec<StockItem>,
    max_listing_date: Option<&str>,
    exclude_boards: &[String],
) -> (Vec<StockItem>, usize, usize) {
    let mut filtered = Vec::new();
    let mut excluded_date = 0;
    let mut excluded_board = 0;

    for item in items {
        if let Some(max_d) = max_listing_date {
            let date_prefix = &item.listing_date[..item.listing_date.len().min(10)];
            if date_prefix > max_d {
                excluded_date += 1;
                continue;
            }
        }

        if !exclude_boards.is_empty() {
            let board = item.listing_board.as_deref().unwrap_or("");
            let matches_exclude = exclude_boards
                .iter()
                .any(|ex| ex.eq_ignore_ascii_case(board));
            if matches_exclude {
                excluded_board += 1;
                continue;
            }
        }

        filtered.push(item);
    }

    (filtered, excluded_date, excluded_board)
}

// -------------------------------------------------------------------------
// Subcommand 3: Market Data & Asymmetry Proxies (Tahap D)
// -------------------------------------------------------------------------

struct MarketArgs {
    input: PathBuf,
    ticker: Option<String>,
    year: Option<u32>,
    format: String,
    output: Option<PathBuf>,
    shares_csv: Option<PathBuf>,
}

fn parse_market_args(args: &[String]) -> Result<MarketArgs, Box<dyn Error>> {
    let (m, _) = parse_flags(
        args,
        &[
            "-i",
            "--input",
            "-t",
            "--ticker",
            "-y",
            "--year",
            "--format",
            "-o",
            "--output",
            "--shares-csv",
        ],
    );

    if m.contains_key("-h") || m.contains_key("--help") {
        eprintln!("usage: idx-prism market -i <file.json|file.csv|dir> [-t <TICKER>] [-y <YEAR>] [--format csv|json] [-o <output_file>] [--shares-csv <ticker,year,shares>]");
        process::exit(0);
    }

    let input = flag(&m, &["-i", "--input"])
        .map(PathBuf::from)
        .ok_or("Error: -i / --input <file|dir> is required")?;

    Ok(MarketArgs {
        input,
        ticker: flag(&m, &["-t", "--ticker"]).map(str::to_uppercase),
        year: flag(&m, &["-y", "--year"]).and_then(|v| v.parse::<u32>().ok()),
        format: flag(&m, &["--format"])
            .map(str::to_lowercase)
            .unwrap_or_else(|| "csv".to_string()),
        output: flag(&m, &["-o", "--output"]).map(PathBuf::from),
        shares_csv: flag(&m, &["--shares-csv"]).map(PathBuf::from),
    })
}

fn run_market(raw_args: &[String]) -> Result<(), Box<dyn Error>> {
    let args = parse_market_args(raw_args)?;
    let shares_map = match &args.shares_csv {
        Some(p) => load_shares_csv(p)?,
        None => std::collections::HashMap::new(),
    };
    let metrics = process_market_path(&args.input, args.ticker.as_deref(), args.year, &shares_map)?;

    if metrics.is_empty() {
        eprintln!("Warning: No valid daily trading data found in input.");
    }

    let out_str = match args.format.as_str() {
        "json" => serde_json::to_string_pretty(&metrics)?,
        _ => {
            let mut s = String::from("ticker,year,trading_days,simple_spread,corwin_schultz_spread,amihud_illiquidity,zero_return_days,volatility,turnover,annual_volume\n");
            for m in &metrics {
                s.push_str(&format!(
                    "{},{},{},{:.6},{:.6},{:.6e},{:.6},{:.6},{},{}\n",
                    m.ticker,
                    m.year,
                    m.trading_days,
                    m.simple_spread,
                    m.corwin_schultz_spread,
                    m.amihud_illiquidity,
                    m.zero_return_days,
                    m.volatility,
                    m.turnover.map(|t| format!("{:.6}", t)).unwrap_or_default(),
                    m.annual_volume
                ));
            }
            s
        }
    };

    if let Some(ref out_path) = args.output {
        fs::write(out_path, &out_str)?;
        eprintln!("Market metrics saved to {}", out_path.display());
    } else {
        print!("{}", out_str);
    }

    Ok(())
}

fn load_shares_csv(
    path: &Path,
) -> Result<std::collections::HashMap<(String, u32), f64>, Box<dyn Error>> {
    let text = fs::read_to_string(path)?;
    let mut map = std::collections::HashMap::new();
    for line in text.lines() {
        let p: Vec<&str> = line.split(',').map(str::trim).collect();
        if p.len() < 3 {
            continue;
        }
        // header line skipped naturally: year/shares won't parse
        let (Ok(year), Ok(shares)) = (p[1].parse::<u32>(), p[2].parse::<f64>()) else {
            continue;
        };
        if shares > 0.0 {
            map.insert((p[0].to_uppercase(), year), shares);
        }
    }
    Ok(map)
}

fn process_market_path(
    path: &Path,
    filter_ticker: Option<&str>,
    filter_year: Option<u32>,
    shares_map: &std::collections::HashMap<(String, u32), f64>,
) -> Result<Vec<MarketMetrics>, Box<dyn Error>> {
    let mut files = Vec::new();
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let p = entry.path();
            if p.is_file() {
                let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
                if ext == "json" || ext == "csv" {
                    files.push(p);
                }
            }
        }
    } else {
        files.push(path.to_path_buf());
    }

    files.sort();
    let mut all_metrics = Vec::new();

    for f in files {
        let (file_ticker, bars) = if f.extension().and_then(|s| s.to_str()) == Some("json") {
            let bytes = fs::read(&f)?;
            parse_yahoo_json(&bytes)?
        } else {
            let text = fs::read_to_string(&f)?;
            let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("UNKNOWN");
            parse_daily_csv(&text, stem)?
        };

        let target_ticker = filter_ticker.unwrap_or(&file_ticker);
        let shares_for = |y: u32| shares_map.get(&(target_ticker.to_uppercase(), y)).copied();

        let mut years = std::collections::BTreeSet::new();
        for b in &bars {
            if b.year > 0 {
                years.insert(b.year);
            }
        }

        if let Some(y) = filter_year {
            let year_bars: Vec<DailyBar> = bars
                .into_iter()
                .filter(|b| b.year == y || b.year == 0)
                .collect();
            if let Some(m) = compute_market_metrics(target_ticker, y, &year_bars, shares_for(y)) {
                all_metrics.push(m);
            }
        } else if !years.is_empty() {
            for y in years {
                let year_bars: Vec<DailyBar> =
                    bars.iter().filter(|b| b.year == y).cloned().collect();
                if let Some(m) = compute_market_metrics(target_ticker, y, &year_bars, shares_for(y))
                {
                    all_metrics.push(m);
                }
            }
        } else {
            return Err(format!(
                "bars in {} have no parseable dates; cannot assign a year",
                f.display()
            )
            .into());
        }
    }

    Ok(all_metrics)
}

fn compute_market_metrics(
    ticker: &str,
    year: u32,
    bars: &[DailyBar],
    shares_outstanding: Option<f64>,
) -> Option<MarketMetrics> {
    if bars.is_empty() {
        return None;
    }

    let mut simple_spreads = Vec::with_capacity(bars.len());
    let mut cs_spreads = Vec::with_capacity(bars.len());
    let mut amihuds = Vec::with_capacity(bars.len());
    let mut returns = Vec::with_capacity(bars.len());
    let mut zero_ret_days = 0usize;
    let mut ret_days = 0usize;
    let mut total_vol: u64 = 0;

    let c_const = 3.0 - 2.0 * 2.0_f64.sqrt();

    for i in 0..bars.len() {
        let b = &bars[i];
        total_vol += b.volume;
        if b.high > 0.0 && b.low > 0.0 && b.high >= b.low && (b.high + b.low) > 0.0 {
            let s = 2.0 * (b.high - b.low) / (b.high + b.low);
            simple_spreads.push(s);
        }

        if i > 0 {
            let prev = &bars[i - 1];
            if prev.close > 0.0 && b.close > 0.0 {
                let ret = (b.close - prev.close) / prev.close;
                returns.push(ret);
                ret_days += 1;
                if ret == 0.0 {
                    zero_ret_days += 1;
                }
                if b.volume > 0 {
                    let dollar_vol = b.close * (b.volume as f64);
                    amihuds.push(ret.abs() / dollar_vol);
                }
            }

            // Corwin & Schultz (2012) 2-day high-low bid-ask spread estimator
            if prev.high > 0.0
                && prev.low > 0.0
                && prev.high >= prev.low
                && b.high > 0.0
                && b.low > 0.0
                && b.high >= b.low
            {
                let beta = (prev.high / prev.low).ln().powi(2) + (b.high / b.low).ln().powi(2);
                let h2 = prev.high.max(b.high);
                let l2 = prev.low.min(b.low);
                let gamma = (h2 / l2).ln().powi(2);

                let alpha =
                    ((2.0 * beta).sqrt() - beta.sqrt()) / c_const - (gamma / c_const).sqrt();
                let cs = if alpha <= 0.0 {
                    0.0
                } else {
                    let exp_a = alpha.exp();
                    2.0 * (exp_a - 1.0) / (1.0 + exp_a)
                };
                cs_spreads.push(cs);
            }
        }
    }

    let trading_days = simple_spreads.len();
    if trading_days == 0 {
        return None;
    }

    let avg_simple = simple_spreads.iter().sum::<f64>() / (trading_days as f64);
    let avg_cs = if cs_spreads.is_empty() {
        0.0
    } else {
        cs_spreads.iter().sum::<f64>() / (cs_spreads.len() as f64)
    };
    let avg_amihud = if amihuds.is_empty() {
        0.0
    } else {
        amihuds.iter().sum::<f64>() / (amihuds.len() as f64)
    };

    // ZRD (VG 2019): zero-return days / total return days
    let zrd = if ret_days > 0 {
        zero_ret_days as f64 / ret_days as f64
    } else {
        0.0
    };

    // VOLATILITY: sample std dev of daily returns (Ln applied downstream in EViews/Stata)
    let volatility = if returns.len() > 1 {
        let mean = returns.iter().sum::<f64>() / returns.len() as f64;
        let var =
            returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (returns.len() - 1) as f64;
        var.sqrt()
    } else {
        0.0
    };

    // TURNOVER (VG 2019): mean daily (shares traded / shares outstanding)
    let turnover = shares_outstanding
        .filter(|s| *s > 0.0)
        .map(|so| (total_vol as f64 / bars.len() as f64) / so);

    Some(MarketMetrics {
        ticker: ticker.to_uppercase(),
        year,
        trading_days,
        simple_spread: avg_simple,
        corwin_schultz_spread: avg_cs,
        amihud_illiquidity: avg_amihud,
        zero_return_days: zrd,
        volatility,
        turnover,
        annual_volume: total_vol,
    })
}

// -------------------------------------------------------------------------
// Market Data Parsers (Yahoo JSON & Daily CSV)
// -------------------------------------------------------------------------

#[derive(Deserialize)]
struct YahooChartResponse {
    chart: YahooChart,
}

#[derive(Deserialize)]
struct YahooChart {
    result: Option<Vec<YahooChartResult>>,
}

#[derive(Deserialize)]
struct YahooChartResult {
    meta: Option<YahooMeta>,
    timestamp: Option<Vec<i64>>,
    indicators: Option<YahooIndicators>,
}

#[derive(Deserialize)]
struct YahooMeta {
    symbol: Option<String>,
}

#[derive(Deserialize)]
struct YahooIndicators {
    quote: Option<Vec<YahooQuote>>,
}

#[derive(Deserialize)]
struct YahooQuote {
    #[serde(default)]
    high: Option<Vec<Option<f64>>>,
    #[serde(default)]
    low: Option<Vec<Option<f64>>>,
    #[serde(default)]
    close: Option<Vec<Option<f64>>>,
    #[serde(default)]
    volume: Option<Vec<Option<u64>>>,
}

fn parse_yahoo_json(bytes: &[u8]) -> Result<(String, Vec<DailyBar>), Box<dyn Error>> {
    let resp: YahooChartResponse = serde_json::from_slice(bytes)?;
    let result = resp.chart.result.ok_or("no chart result in Yahoo JSON")?;
    let first = result
        .into_iter()
        .next()
        .ok_or("empty chart result array")?;
    let symbol = first
        .meta
        .and_then(|m| m.symbol)
        .unwrap_or_else(|| "UNKNOWN".into());
    let ticker = symbol.split('.').next().unwrap_or(&symbol).to_uppercase();

    let timestamps = first.timestamp.unwrap_or_default();
    let quote = first
        .indicators
        .and_then(|ind| ind.quote)
        .and_then(|q| q.into_iter().next())
        .ok_or("no quote indicators in Yahoo JSON")?;

    let highs = quote.high.unwrap_or_default();
    let lows = quote.low.unwrap_or_default();
    let closes = quote.close.unwrap_or_default();
    let volumes = quote.volume.unwrap_or_default();

    let n = timestamps.len();
    let mut bars = Vec::with_capacity(n);

    for (i, &ts) in timestamps.iter().enumerate() {
        let year = epoch_to_year(ts);
        let h = highs.get(i).and_then(|&x| x).unwrap_or(0.0);
        let l = lows.get(i).and_then(|&x| x).unwrap_or(0.0);
        let c = closes.get(i).and_then(|&x| x).unwrap_or(0.0);
        let v = volumes.get(i).and_then(|&x| x).unwrap_or(0);

        if h > 0.0 && l > 0.0 && c > 0.0 && h >= l {
            bars.push(DailyBar {
                year,
                high: h,
                low: l,
                close: c,
                volume: v,
            });
        }
    }

    Ok((ticker, bars))
}

fn parse_daily_csv(
    content: &str,
    default_ticker: &str,
) -> Result<(String, Vec<DailyBar>), Box<dyn Error>> {
    let mut lines = content.lines();
    let header = lines.next().ok_or("empty CSV")?;
    let cols: Vec<String> = header.split(',').map(|s| s.trim().to_lowercase()).collect();

    let idx_date = cols.iter().position(|c| c.contains("date")).unwrap_or(0);
    let idx_high = cols.iter().position(|c| c == "high").unwrap_or(2);
    let idx_low = cols.iter().position(|c| c == "low").unwrap_or(3);
    let idx_close = cols
        .iter()
        .position(|c| c == "close" || c == "adj close")
        .unwrap_or(4);
    let idx_vol = cols.iter().position(|c| c.contains("vol")).unwrap_or(5);

    let mut bars = Vec::new();
    for line in lines {
        let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if parts.len() <= idx_close {
            continue;
        }
        let date_str = parts.get(idx_date).copied().unwrap_or("");
        let year = if date_str.len() >= 4 {
            date_str[..4].parse::<u32>().unwrap_or(0)
        } else {
            0
        };

        let h = parts
            .get(idx_high)
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let l = parts
            .get(idx_low)
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let c = parts
            .get(idx_close)
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let v = parts
            .get(idx_vol)
            .and_then(|s| s.parse::<f64>().ok())
            .map(|x| x as u64)
            .unwrap_or(0);

        if h > 0.0 && l > 0.0 && c > 0.0 && h >= l {
            bars.push(DailyBar {
                year,
                high: h,
                low: l,
                close: c,
                volume: v,
            });
        }
    }

    Ok((default_ticker.to_uppercase(), bars))
}

fn epoch_to_year(ts: i64) -> u32 {
    let days = ts / 86400;
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1020 + doe / 1461 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let final_y = if m <= 2 { y + 1 } else { y };
    final_y as u32
}

// -------------------------------------------------------------------------
// Core XBRL & XML Parsers
// -------------------------------------------------------------------------

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
                        b"SalesAndRevenue" | b"Revenues"
                            if is_current_duration && report.revenues.is_none() =>
                        {
                            report.revenues = read_text_num(&mut reader)?;
                        }
                        b"ProfitLoss" | b"NetIncomeLoss"
                            if is_current_duration && report.net_income.is_none() =>
                        {
                            report.net_income = read_text_num(&mut reader)?;
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

    // ponytail: regex fallback karena quick-xml miss tag idx-cor:InvestmentPropertiesTextBlock.
    // Satu alternation menangani tag plural & singular; ambil match pertama yang non-kosong
    // (sebagian emiten menaruh tag kosong sebelum tag berisi).
    let re = Regex::new(
        r"(?s)<(?:[\w-]+:)?InvestmentPropert(?:y|ies)TextBlock[^>]*>(.*?)</(?:[\w-]+:)?InvestmentPropert(?:y|ies)TextBlock>",
    )
    .ok()?;

    let raw = re
        .captures_iter(&text)
        .filter_map(|m| m.get(1))
        .map(|m| m.as_str())
        .find(|raw| !raw.trim().is_empty());

    raw.map(clean_policy_text)
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

/// Klasifikasi model pengukuran properti investasi (PSAK 240).
///
/// Akar masalah versi lama: mencocokkan KATA KUNCI ("biaya"/"cost") alih-alih
/// KALIMAT PENGUKURAN. Setiap emiten model nilai wajar menulis "biaya perolehan
/// pada saat pengakuan awal" (biaya perolehan awal ≠ model biaya), sehingga
/// MMLP/KBAG/BSBK — yang jelas fair value — terklasifikasi `cost model`.
/// Arah salah ini berbahaya: emiten FV akan masuk sampel cost-model (kriteria 3).
///
/// Karena itu: cari kalimat pengukuran SETELAH pengakuan awal; di situ modelnya
/// dinyatakan. Kalau tidak ada penanda itu, baru pakai indikator seluruh teks —
/// dan indikator biaya harus POSITIF (penyusutan/akumulasi/model biaya), bukan
/// sekadar kata "biaya".
fn detect_model(policy: &str) -> String {
    let mut lower = policy.to_lowercase();
    // Negasi: "tidak disusutkan" / "not depreciated" adalah penanda MODEL NILAI
    // WAJAR (FV tidak disusutkan), bukan penanda biaya. Tanpa ini, PLIN
    // (yang menyatakan "menggunakan model nilai wajar") terbaca `cost model`.
    for neg in [
        "tidak disusutkan",
        "tidak diamortisasi",
        "tidak mengalami penyusutan",
        "not depreciated",
        "not amortised",
        "not amortized",
        "no depreciation",
    ] {
        lower = lower.replace(neg, " ");
    }

    // Indikator kuat, dipakai baik di jendela maupun seluruh teks.
    let cost_strong = [
        "model biaya",
        "cost model",
        "akumulasi penyusutan",
        "accumulated depreciation",
        "disusutkan",
        "depreciated",
        "garis lurus",
        "straight-line",
        "straight line",
        "umur manfaat",
        "useful life",
    ];
    // "model revaluasi" = keluarga nilai wajar, tapi bukan fair value model murni
    // (dipakai KBAG/BSBK sebelum beralih). Dibedakan agar tabel seleksi jujur.
    let reval = ["model revaluasi", "revaluation model"];
    let fv = [
        "model nilai wajar",
        "fair value model",
        "nilai wajarnya",
        "nilai wajar",
        "fair value",
    ];
    let has = |w: &str, pats: &[&str]| pats.iter().any(|p| w.contains(p));

    // Kalimat kebijakan yang MENENTUKAN model saat ini. Emiten yang beralih model
    // menyebut keduanya secara kronologis (EMDE: "sebelum 1 Jan 2021 … model
    // biaya … mulai 1 Jan 2021 … model nilai wajar"), jadi yang dipakai adalah
    // kemunculan TERAKHIR — kebijakan terbaru ditulis belakangan.
    // ponytail: bergantung urutan kronologis dalam catatan. Kalau ada emiten yang
    // menulis kebijakan terkini lebih dulu, cross-check PDF/vision (Fase 4).
    let mut governing: Option<(usize, &str)> = None;
    for (a, model) in [
        ("menggunakan model nilai wajar", "fair value model"),
        ("uses the fair value model", "fair value model"),
        ("menggunakan model biaya", "cost model"),
        ("uses the cost model", "cost model"),
    ] {
        if let Some(i) = lower.rfind(a) {
            if governing.is_none_or(|(pos, _)| i > pos) {
                governing = Some((i, model));
            }
        }
    }
    if let Some((_, model)) = governing {
        return model.to_string();
    }

    // Jendela ~260 char setelah penanda pengukuran lanjutan: di situlah model
    // dinyatakan ("setelah pengakuan awal, ... dicatat pada nilai wajar" /
    // "... berdasarkan model biaya ... disusutkan").
    let anchors = [
        "setelah pengakuan awal",
        "subsequent to initial recognition",
        "subsequently measured",
        "selanjutnya diukur",
        "diukur selanjutnya",
        "dicatat menggunakan model",
    ];
    let mut window = String::new();
    for a in anchors {
        if let Some(i) = lower.find(a) {
            let end = (i + a.len() + 260).min(lower.len());
            window.push_str(&lower[i..end]);
            window.push(' ');
        }
    }

    if !window.is_empty() {
        // Biaya diperiksa LEBIH DULU: ARGO menyebut "diukur selanjutnya pada
        // nilai wajar" di blok definisi, tapi kalimat pengukurannya model biaya.
        if has(&window, &cost_strong) {
            return "cost model".to_string();
        }
        if has(&window, &reval) {
            return "revaluation model".to_string();
        }
        if has(&window, &fv) {
            return "fair value model".to_string();
        }
    }

    // Tanpa penanda pengukuran lanjutan: indikator kuat di seluruh teks.
    if has(&lower, &cost_strong) {
        return "cost model".to_string();
    }
    if has(&lower, &reval) {
        return "revaluation model".to_string();
    }
    // Frasa FV harus berupa pengukuran, bukan sekadar penyebutan; "nilai wajar"
    // sendirian terlalu longgar (definisi & kombinasi bisnis memuatnya), jadi
    // hanya frasa pengukuran eksplisit yang diterima.
    let fv_measure = [
        "diukur dengan menggunakan nilai wajar",
        "diukur pada nilai wajar",
        "diukur sebesar nilai wajar",
        "dicatat pada nilai wajar",
        "dicatat sebesar nilai wajar",
        "dinilai sebesar nilai wajar",
        "dinyatakan berdasarkan nilai wajar",
        "measured at fair value",
        "model nilai wajar",
        "fair value model",
    ];
    if has(&lower, &fv_measure) {
        return "fair value model".to_string();
    }
    "unknown".to_string()
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
        /// `(public_shares, total_shares, free_float_pct)` — all-or-nothing: the
        /// strict cross-check in `extract_ownership` emits nothing unless every
        /// component was verified, so a partial tuple never occurs.
        pub ownership: Option<(i64, i64, f64)>,
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
            ownership: extract_ownership(&text),
        })
    }

    /// Below this, `extract_words` already merges runs that share a baseline;
    /// this is the page-level tolerance for deciding two words are on the same
    /// visual row. Insensitive in the 1.0–4.0 range on the pilot corpus, and
    /// set to pdf_oxide's own table-detector `row_tolerance` default.
    const ROW_TOL: f32 = 2.8;

    /// Rebuild a page's text as visual rows.
    ///
    /// `extract_text` emits words in content-stream order, which splits a table
    /// row across several lines — "Masyarakat" on one line, "8.637.784.479
    /// 46,60%" on the next — and defeats row-anchored parsing (CTRA, SMRA,
    /// APLN all failed this way). Grouping by word-box `y` and sorting within a
    /// row by `x` restores the row, so downstream line regexes see what a
    /// human sees.
    ///
    /// ponytail: linear scan per word, O(words x rows) per page (~18k compares
    /// on a dense page). Switch to a sorted sweep if a page ever gets slow.
    fn page_rows_to_text(words: &[pdf_oxide::layout::Word]) -> String {
        let mut rows: Vec<(f32, Vec<(f32, &str)>)> = Vec::new();
        for w in words {
            let y = w.bbox.y;
            match rows.iter_mut().find(|(ry, _)| (*ry - y).abs() <= ROW_TOL) {
                Some((_, v)) => v.push((w.bbox.x, w.text.as_str())),
                None => rows.push((y, vec![(w.bbox.x, w.text.as_str())])),
            }
        }
        // y grows upward in PDF space: descending y == top-to-bottom reading order
        rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let mut out = String::new();
        for (_, mut row) in rows {
            row.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
            for (i, (_, t)) in row.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                out.push_str(t);
            }
            out.push('\n');
        }
        out
    }

    fn extract_pdf_text<P: AsRef<Path>>(path: P) -> Result<String, Box<dyn Error>> {
        use pdf_oxide::PdfDocument;
        let doc = PdfDocument::open(path.as_ref())?;
        let mut text = String::new();
        let page_count = doc.page_count()?;
        for i in 0..page_count {
            if let Ok(words) = doc.extract_words(i) {
                text.push_str(&page_rows_to_text(&words));
                text.push('\n');
            }
        }
        Ok(text)
    }

    fn investment_property_region<'a>(text: &'a str, lower: &'a str) -> &'a str {
        // Anchor order matters: descriptive phrase, then the numbered CALK note
        // anchor (note numbers differ per emiten — 10/13/14/16 observed — so match
        // any number instead of listing them), then the catch-alls.
        let numbered =
            Regex::new(r"\d+\.\s*(?:properti investasi|investment propert(?:y|ies))").ok();

        let pos = [
            "nilai wajar properti investasi",
            "fair values of certain investment properties",
        ]
        .iter()
        .find_map(|m| lower.find(m))
        .or_else(|| numbered.as_ref()?.find(lower).map(|m| m.start()))
        .or_else(|| lower.find("properti investasi - neto"))
        .or_else(|| lower.find("properti investasi"));

        match pos {
            Some(p) => &text[p..text.len().min(p + 35000)],
            None => text,
        }
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

        // fallback longgar: fragmen setelah frasa. Urutan pola penting (ID dulu, lalu EN) —
        // jangan digabung jadi satu alternation, karena alternation menang berdasarkan
        // posisi di teks, bukan prioritas pola.
        for pat in [
            r"(?i)nilai wajar properti investasi[^.]{0,150}",
            r"(?i)fair value of (?:the )?investment propert(?:y|ies)[^.,]{0,150}",
        ] {
            if let Some(m) = Regex::new(pat).ok()?.find(text) {
                return Some(m.as_str().trim().to_string());
            }
        }

        None
    }

    fn extract_appraiser_name(text: &str) -> Option<String> {
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

    // Komposisi kepemilikan saham (CALK note Modal Saham / Capital Stock).
    // pdf_oxide mengekstrak tabel ini ROW-MAJOR: satu baris = satu pemegang saham
    // dengan jumlah saham + persentase inline, mis.:
    //   "Masyarakat (masing-masing dibawah 5%) 15.070.264.960 31,30 376.756.624 Public ..."
    //   "Jumlah 48.159.602.400 100,00 1.203.990.060 Total"
    // Algoritma: anchor ke note "Modal Saham/Capital Stock", cari baris publik
    // (keyword + jumlah saham dotted + persen koma inline) & baris Jumlah ~100%.
    // STRICT GATE: emit hanya bila cross-check public/total*100 == free_float (<=0.5pp).
    // Layout yang pecah (nama & angka terpisah baris) sudah ditangani
    // `page_rows_to_text`; di sini yang tersisa adalah memilih BARIS YANG BENAR,
    // karena `\bpublic\b` juga cocok pada prosa dwibahasa ("a public notary",
    // "Notice of Effectivity") yang tersebar jauh sebelum tabel Modal Saham.
    pub(crate) fn extract_ownership(text: &str) -> Option<(i64, i64, f64)> {
        // jumlah saham: integer besar ber-titik; persen: desimal koma (gaya Indonesia)
        let re_share = Regex::new(r"(\d{1,3}(?:\.\d{3})+)").unwrap();
        let re_pct = Regex::new(r"(\d{1,3},\d+)").unwrap();
        // persen bulat: hanya bila diikuti '%' — tanpa itu, angka biasa seperti
        // tahun atau jumlah saham ikut cocok. Dipakai KHUSUS untuk baris total,
        // yang sebagian emiten tulis "100%" alih-alih "100,00" (CTRA).
        let re_pct_whole = Regex::new(r"(\d{1,3})\s*%").unwrap();
        // Label publik HARUS mengawali barisnya sendiri. Versi longgar
        // (`\bpublic\b` di mana saja) juga cocok pada prosa dwibahasa
        // ("a public notary", "Notice of Effectivity") sehingga baris direksi
        // ikut terlabeli — itulah yang menghasilkan `Harun Hajadi ... 0,08%`.
        let re_public_line =
            Regex::new(r"(?i)^\s*(masyarakat|publik|lain-lain|others|public)\b").unwrap();
        // Baris manajemen (dan sub-headernya) bukan pemegang publik, apa pun
        // label di atasnya. CTRA: "Lain-lain (…kurang dari 5%)" adalah header
        // KELOMPOK, di bawahnya ada sub-header "Manajemen:" lalu baris-baris
        // direktur — sehingga baris direktur tampak "berlabel publik".
        let re_manager =
            Regex::new(r"(?i)direksi|komisaris|director|commissioner|manajemen|management")
                .unwrap();
        let re_total = Regex::new(r"(?i)\bjumlah\b|\btotal\b|\bsub-total\b").unwrap();

        let lines: Vec<&str> = text.lines().map(|l| l.trim()).collect();
        let n = lines.len();

        let first_share = |l: &str| -> Option<i64> {
            re_share
                .captures(l)
                .and_then(|c| c[1].replace('.', "").parse::<i64>().ok())
                .filter(|v| *v >= 1000)
        };
        let first_pct = |l: &str| -> Option<f64> {
            re_pct
                .captures(l)
                .and_then(|c| c[1].replace(',', ".").parse::<f64>().ok())
        };
        // baris total: persen koma lebih diutamakan; kalau tidak ada, terima "100%"
        let total_pct = |l: &str| -> Option<f64> {
            first_pct(l).or_else(|| {
                re_pct_whole
                    .captures(l)
                    .and_then(|c| c[1].parse::<f64>().ok())
            })
        };
        let inline_row = |l: &str| -> bool { first_share(l).is_some() && first_pct(l).is_some() };
        let inline_total = |l: &str| -> bool { first_share(l).is_some() && total_pct(l).is_some() };

        // Label publik kadang terlipat ke baris di atasnya, sehingga baris angka
        // sendiri tak memuat kata kunci: APLN "Masyarakat umum" satu baris di
        // atas; SMRA "Lain-lain (masing-masing / dengan pemilikan kurang"
        // dua baris di atas. Baris angka tetap satu baris (berkat
        // page_rows_to_text); hanya labelnya yang melipat.
        // Syaratnya: baris label harus MENGAWALI dengan kata kunci, dan semua
        // baris antara label dan baris angka harus bebas angka (murni lanjutan
        // label) — bukan sekadar "ada kata kunci dalam radius 3 baris".
        const LABEL_LOOKBACK: usize = 3;
        // Baris label harus baris label MURNI (tanpa saham+persen inline).
        // Kalau tidak, baris Total yang tepat berada di bawah baris publik
        // ("Masyarakat 8.637.784.479 46,60%") akan lolos sebagai kandidat
        // dengan persen 100 — itulah yang membuat DILD mengembalikan 100,0.
        let label_above = |i: usize| -> bool {
            (1..=LABEL_LOOKBACK.min(i)).any(|k| {
                let l = lines[i - k];
                re_public_line.is_match(l)
                    && !inline_row(l)
                    && lines[(i - k + 1)..i]
                        .iter()
                        .all(|m| !inline_row(m) && !re_manager.is_match(m))
            })
        };

        // Kandidat PERTAMA yang lolos dipakai, bukan persen terbesar: halaman
        // memuat tabel tahun berjalan LALU tahun sebelumnya, dan angka tahun
        // sebelumnya bisa lebih besar (BSDE 30,00 vs 28,94) sehingga "terbesar"
        // memilih tahun yang salah. Versi lama juga memakai kandidat pertama —
        // yang salah di CTRA (0,08) bukan karena urutannya, melainkan karena
        // baris direktur ikut lolos gate. Sekarang baris itu diblokir gerbang
        // identitas di atas, jadi urutan dokumen kembali bermakna.
        // ponytail: mengandalkan urutan dokumen (tahun berjalan dulu). Kalau ada
        // emiten yang membalik urutan, jalur vision (Fase 3-5) yang cross-check.
        for i in 0..n {
            // baris total/manajemen bukan baris pemegang publik
            if !inline_row(lines[i]) || re_manager.is_match(lines[i]) || re_total.is_match(lines[i])
            {
                continue;
            }
            if !(re_public_line.is_match(lines[i]) || label_above(i)) {
                continue;
            }

            // baris Jumlah/Total INLINE terdekat setelahnya (<=40 baris)
            let Some(ti) = lines[(i + 1)..(i + 41).min(n)]
                .iter()
                .position(|l| re_total.is_match(l) && inline_total(l))
                .map(|k| i + 1 + k)
            else {
                continue;
            };

            let (Some(public_shares), Some(free_float), Some(total_shares), Some(total_pct)) = (
                first_share(lines[i]),
                first_pct(lines[i]),
                first_share(lines[ti]),
                total_pct(lines[ti]),
            ) else {
                continue;
            };

            // total row harus ~100%; cross-check rasio publik == free_float
            let total_ok = (99.5..=100.05).contains(&total_pct);
            let xcheck_ok = total_shares > 0
                && (100.0 * public_shares as f64 / total_shares as f64 - free_float).abs() <= 0.5;

            if total_ok && xcheck_ok {
                return Some((public_shares, total_shares, free_float));
            }
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

    #[test]
    fn test_filter_securities() {
        let items = vec![
            StockItem {
                code: "CTRA".into(),
                name: "Ciputra".into(),
                listing_date: "1994-03-28T00:00:00".into(),
                shares: None,
                listing_board: Some("Utama".into()),
            },
            StockItem {
                code: "TRUE".into(),
                name: "Triniti".into(),
                listing_date: "2021-06-10T00:00:00".into(),
                shares: None,
                listing_board: Some("Pengembangan".into()),
            },
            StockItem {
                code: "IPAC".into(),
                name: "Era".into(),
                listing_date: "2020-01-01T00:00:00".into(),
                shares: None,
                listing_board: Some("Akselerasi".into()),
            },
        ];

        let (filtered, excl_date, excl_board) =
            filter_securities(items, Some("2021-01-01"), &["Akselerasi".to_string()]);

        assert_eq!(excl_date, 1);
        assert_eq!(excl_board, 1);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].code, "CTRA");
    }

    #[test]
    fn test_market_metrics_calculation() {
        let bars = vec![
            DailyBar {
                year: 2023,
                high: 100.0,
                low: 90.0,
                close: 95.0,
                volume: 1000,
            },
            DailyBar {
                year: 2023,
                high: 105.0,
                low: 95.0,
                close: 100.0,
                volume: 2000,
            },
            DailyBar {
                year: 2023,
                high: 110.0,
                low: 98.0,
                close: 105.0,
                volume: 1500,
            },
        ];

        let m = compute_market_metrics("TEST", 2023, &bars, None).unwrap();
        assert_eq!(m.ticker, "TEST");
        assert_eq!(m.year, 2023);
        assert_eq!(m.trading_days, 3);
        assert!(m.simple_spread > 0.0);
        assert!(m.corwin_schultz_spread >= 0.0);
        assert!(m.amihud_illiquidity > 0.0);
        assert_eq!(m.annual_volume, 4500);
        // ZRD: 2 return-day, close 95->100->105 (tak ada yang nol) = 0
        assert_eq!(m.zero_return_days, 0.0);
        // volatility sample std dev dari [0.05263, 0.05] > 0
        assert!(m.volatility > 0.0);
        // tanpa shares -> turnover None
        assert!(m.turnover.is_none());

        // dengan shares outstanding -> turnover = mean(volume)/shares
        let m2 = compute_market_metrics("TEST", 2023, &bars, Some(9000.0)).unwrap();
        assert!((m2.turnover.unwrap() - (4500.0 / 3.0) / 9000.0).abs() < 1e-9);
    }

    #[test]
    fn test_zrd_counts_zero_return_days() {
        // close 100 -> 100 -> 105 : 1 dari 2 return-day = 0.5
        let bars = vec![
            DailyBar {
                year: 2023,
                high: 101.0,
                low: 99.0,
                close: 100.0,
                volume: 1000,
            },
            DailyBar {
                year: 2023,
                high: 101.0,
                low: 99.0,
                close: 100.0,
                volume: 1000,
            },
            DailyBar {
                year: 2023,
                high: 106.0,
                low: 100.0,
                close: 105.0,
                volume: 1000,
            },
        ];
        let m = compute_market_metrics("Z", 2023, &bars, None).unwrap();
        assert_eq!(m.zero_return_days, 0.5);
    }

    #[test]
    fn test_parse_policy_text_skips_idem_reference() {
        // CTRA 2023: tag singular berisi kebijakan riil, tag plural hanya penunjuk
        // "Idem row 10". Ambil yang riil (urutan dokumen), bukan penunjuknya —
        // kalau tidak, extractor jatuh ke teks PDF yang belum dinormalisasi.
        let xml = br#"<idx-cor:InvestmentPropertyTextBlock ctx="a">Properti investasi diukur dengan model biaya</idx-cor:InvestmentPropertyTextBlock><idx-cor:InvestmentPropertiesTextBlock ctx="b">Idem row 10</idx-cor:InvestmentPropertiesTextBlock>"#;
        assert_eq!(
            parse_policy_text(xml).as_deref(),
            Some("Properti investasi diukur dengan model biaya")
        );
    }

    #[test]
    fn test_extract_ownership_row_major() {
        // meniru output pdf_oxide PWON (row-major: nama + saham + persen inline per baris)
        let txt = "22. MODAL SAHAM 22. CAPITAL STOCK\n\
Nama Pemegang Saham Shares of Ownership\n\
PT Pakuwon Arthaniaga 33.077.598.400 68,68 826.939.960 PT Pakuwon Arthaniaga\n\
Alexander Tedja 10.608.000 0,02 265.200 Alexander Tedja\n\
Wong Boon Siew Ivy 1.000.000 0,00 25.000 Wong Boon Siew Ivy\n\
Richard Adisastra 131.040 0,00 3.276 Richard Adisastra\n\
Masyarakat (masing-masing dibawah 5%) 15.070.264.960 31,30 376.756.624 Public (less than 5% each)\n\
Jumlah 48.159.602.400 100,00 1.203.990.060 Total\n";
        let own = super::pdf_extract::extract_ownership(txt);
        let (public_shares, total_shares, free_float_pct) =
            own.expect("row-major layout must parse");
        assert_eq!(free_float_pct, 31.30);
        assert_eq!(public_shares, 15_070_264_960);
        assert_eq!(total_shares, 48_159_602_400);
    }

    #[test]
    fn test_extract_ownership_ctra_identity_gate() {
        // CTRA 2023 (teks layout-aware dari page_rows_to_text): baris direktur
        // "Harun Hajadi 14.399.560 0,08%" berada DI ATAS baris publik
        // "Masyarakat 8.637.784.479 46,60%". Versi lama mengambil kandidat
        // PERTAMA yang lolos gate -> 0,08 (salah, tapi lolos gate karena
        // 0,08% memang rasio baris itu terhadap total).
        let txt = "Manajemen: Management:\n\
Harun Hajadi 14.399.560 0,08% 3.600 Harun Hajadi\n\
Nanik J. Santoso 857.039 0,00% 214 Nanik J. Santoso\n\
Masyarakat 8.637.784.479 46,60% 2.159.446 Public\n\
Jumlah 18.535.695.255 100% 4.633.924 Total\n";
        let (ps, ts, ff) =
            super::pdf_extract::extract_ownership(txt).expect("CTRA row-major must parse");
        assert_eq!(ff, 46.60);
        assert_eq!(ps, 8_637_784_479);
        assert_eq!(ts, 18_535_695_255);
    }

    #[test]
    fn test_detect_model_transition_uses_current_policy() {
        // EMDE: menyebut kebijakan LAMA lalu BARU secara kronologis. Yang berlaku
        // adalah yang terakhir (model nilai wajar), bukan "model biaya" historis.
        let t =
            "Sebelum tanggal 1 Januari 2021, properti investasi diukur menggunakan model biaya \
                 untuk pengukuran setelah pengakuan. Berdasarkan model biaya, properti investasi \
                 diukur sebesar nilai perolehan setelah dikurangi akumulasi penyusutan. \
                 Mulai tanggal 1 Januari 2021, Grup menggunakan model nilai wajar untuk pengukuran \
                 setelah pengakuan.";
        assert_eq!(detect_model(t), "fair value model");
    }

    #[test]
    fn test_detect_model_negated_depreciation_is_fair_value() {
        // PLIN: satu-satunya token mirip-biaya adalah "tidak disusutkan" — dan itu
        // justru penanda FV (properti FV tidak disusutkan), bukan penanda biaya.
        let t = "Properti investasi yang penyelesaian masa depannya belum ditentukan \
                 diklasifikasikan sebagai properti investasi dan tidak disusutkan. \
                 Perseroan menggunakan model nilai wajar untuk pengukuran setelah pengakuan.";
        assert_eq!(detect_model(t), "fair value model");
    }

    #[test]
    fn test_detect_model_initial_cost_is_not_cost_model() {
        // MMLP: "biaya perolehan" hanya pada PENGAKUAN AWAL; pengukuran lanjutannya
        // nilai wajar. Versi lama membaca kata "biaya" lalu menyimpulkan cost model —
        // arah salah yang berbahaya (emiten FV masuk sampel cost-model, kriteria 3).
        let t = "Properti investasi pada awalnya diukur sebesar biaya perolehan, termasuk biaya \
                 transaksi dan selanjutnya diukur pada nilai wajarnya.";
        assert_eq!(detect_model(t), "fair value model");
    }

    #[test]
    fn test_detect_model_revaluation_category() {
        // KBAG: kategori ketiga (model revaluasi) — bukan cost, bukan FV murni.
        // Dibedakan supaya tabel seleksi jujur; kriteria 3 tetap mengecualikannya.
        let t = "Properti investasi dicatat menggunakan model revaluasi yaitu nilai wajar pada \
                 tanggal revaluasi.";
        assert_eq!(detect_model(t), "revaluation model");
    }

    #[test]
    fn test_detect_model_cost_wins_over_fv_in_definition() {
        // ARGO: blok definisi menyebut "diukur selanjutnya pada nilai wajar", TAPI
        // kalimat pengukuran kebijakannya model biaya + disusutkan. Karena itu
        // biaya diperiksa lebih dulu di dalam jendela pengukuran.
        let t = "Properti investasi diukur pada harga perolehan pada saat pengakuan awal dan \
                 diukur selanjutnya pada nilai wajar dengan segala perubahannya di dalam laba rugi. \
                 Pengakuan awal properti investasi sebesar biaya perolehan, setelah pengakuan awal \
                 dinyatakan berdasarkan model biaya yang dicatat sebesar biaya perolehan dikurangi \
                 akumulasi penyusutan. Bangunan disusutkan dengan metode garis lurus.";
        assert_eq!(detect_model(t), "cost model");
    }

    #[test]
    fn test_detect_model_fv_measurement_without_anchor() {
        // BBSS/TRIN: menyatakan FV tanpa penanda "setelah pengakuan awal".
        let t = "Properti investasi adalah properti untuk menghasilkan pendapatan sewa atau untuk \
                 kenaikan nilai atau keduanya. Properti investasi diukur dengan menggunakan nilai wajar.";
        assert_eq!(detect_model(t), "fair value model");
    }

    #[test]
    fn test_no_secrets_or_local_paths_in_tracked_files() {
        // Gerbang sanitasi (Fase 7): gagal bila ada kunci API atau jalur mesin
        // pribadi yang bocor ke berkas terlacak. Pola dirakit dari potongan
        // supaya berkas ini sendiri tidak memicu positif palsu.
        let pats = [
            ["s", "k-"].concat(),
            ["B", "ear", "er "].concat(),
            ["api", "_", "key"].concat(),
            ["API", "_", "KEY"].concat(),
            ["HERMES_CUSTOM", "_API"].concat(),
            ["infer", "hub.dev"].concat(),
            ["BEGIN ", "PRIVATE"].concat(),
            ["/home/", "micro", "devil"].concat(),
        ];
        let Ok(o) = process::Command::new("git").args(["ls-files"]).output() else {
            return; // bukan repo git (mis. build dari tarball) -> lewati
        };
        let files = String::from_utf8_lossy(&o.stdout);
        let mut hits: Vec<String> = Vec::new();
        for f in files.lines() {
            let f = f.trim();
            if f.is_empty() {
                continue;
            }
            let Ok(body) = fs::read_to_string(f) else {
                continue; // berkas biner -> tidak diperiksa
            };
            for p in &pats {
                if body.contains(p.as_str()) {
                    hits.push(format!("{} <- {}", f, p));
                }
            }
        }
        assert!(
            hits.is_empty(),
            "kebocoran di berkas terlacak (kunci API / jalur pribadi): {:#?}",
            hits
        );
    }

    #[test]
    fn test_extract_ownership_rejects_total_row_as_candidate() {
        // DILD: baris Total tepat di bawah baris publik. Versi yang membolehkan
        // "label di baris atas" tanpa syarat baris-label-murni menjadikan baris
        // Total sebagai kandidat berlabel (100%) — mengembalikan 100,0.
        let txt = "Lain-lain (masing-masing di bawah 5%)\n\
Masyarakat lainnya 3.788.659.282 36,55% 947.164.821 Others\n\
Jumlah 10.365.854.185 100,00 2.591.463.546 Total\n";
        let (_, _, ff) = super::pdf_extract::extract_ownership(txt).expect("must parse");
        assert_eq!(ff, 36.55, "baris Total tidak boleh jadi kandidat");
    }

    #[test]
    fn test_extract_ownership_prefers_current_year_table() {
        // BSDE: halaman memuat tabel tahun berjalan LALU tahun sebelumnya, dan
        // persen tahun sebelumnya lebih besar (30,00 vs 28,94). Aturan "ambil
        // persen terbesar" memilih tahun yang SALAH; kandidat pertama benar.
        let txt = "Masyarakat/Public 6.053.131.112 28,94% 605.313.111 Public\n\
Jumlah 20.913.395.112 100,00 2.091.339.511 Total\n\
31 Desember/December 31 2022\n\
Masyarakat/Public 6.250.000.000 30,00% 625.000.000 Public\n\
Jumlah 20.833.333.333 100,00 2.083.333.333 Total\n";
        let (_, _, ff) = super::pdf_extract::extract_ownership(txt).expect("must parse");
        assert_eq!(
            ff, 28.94,
            "harus mengambil tabel tahun berjalan, bukan terbesar"
        );
    }

    #[test]
    fn test_extract_ownership_rejects_director_under_group_header() {
        // CTRA: "Lain-lain (…kurang dari 5%)" adalah header KELOMPOK, di bawahnya
        // sub-header "Manajemen:" lalu baris direktur. Baris direktur tidak boleh
        // dianggap "berlabel publik" hanya karena header grup ada 3 baris di atas.
        let txt = "Lain-lain (masing-masing dengan Others (each below\n\
pemilikan kurang dari 5%): 5% ownership):\n\
Manajemen: Management:\n\
Harun Hajadi 14.399.560 0,08% 3.600 Harun Hajadi\n\
Masyarakat 8.637.784.479 46,60% 2.159.446 Public\n\
Jumlah 18.535.695.255 100% 4.633.924 Total\n";
        let (_, _, ff) = super::pdf_extract::extract_ownership(txt).expect("must parse");
        assert_eq!(
            ff, 46.60,
            "baris direktur di bawah header grup harus ditolak"
        );
    }

    #[test]
    fn test_extract_ownership_rejects_split_layout() {
        // layout "pecah" (CTRA/SMRA: nama & angka di baris terpisah) -> baris publik
        // TIDAK punya saham+persen inline -> strict gate menolak (None), bukan angka salah.
        let txt = "MODAL SAHAM\n\
Masyarakat\n\
Lain-lain\n\
8.637.784.479\n\
46,60%\n\
Jumlah\n\
18.535.695.255\n\
100%\n";
        assert_eq!(super::pdf_extract::extract_ownership(txt), None);
    }

    #[test]
    fn test_extract_ownership_rejects_bad_xcheck() {
        // baris publik inline ADA, tapi rasio public/total tidak cocok dgn persen
        // -> cross-check gagal -> None (bunuh false-positive)
        let txt = "MODAL SAHAM\n\
Masyarakat 5.000.000.000 50,00 100.000.000\n\
Jumlah 48.000.000.000 100,00 900.000.000\n";
        // 5e9/48e9 = 10.4% != 50% -> reject
        assert_eq!(super::pdf_extract::extract_ownership(txt), None);
    }
}
