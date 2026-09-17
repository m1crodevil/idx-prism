use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use regex::Regex;
use serde::{Deserialize, Serialize};
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
    let raw_args: Vec<String> = env::args().collect();
    if raw_args.len() > 1 && raw_args[1] == "sector" {
        if let Err(e) = run_sector(&raw_args[2..]) {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    } else if raw_args.len() > 1 && raw_args[1] == "market" {
        if let Err(e) = run_market(&raw_args[2..]) {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    } else {
        if let Err(e) = run_extract() {
            eprintln!("Error: {}", e);
            process::exit(1);
        }
    }
}

// -------------------------------------------------------------------------
// Subcommand 1: Financial & CALK Extract
// -------------------------------------------------------------------------

fn run_extract() -> Result<(), Box<dyn Error>> {
    let args = parse_extract_args();

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
        report.public_shares = extracted.ownership.public_shares;
        report.shares_outstanding = extracted.ownership.total_shares;
        report.free_float_pct = extracted
            .ownership
            .free_float_pct
            .map(|p| (p * 100.0).round() / 100.0);

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

fn parse_extract_args() -> ExtractArgs {
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
        eprintln!("usage: idx-prism <ticker> -f <instance.zip> -y <year> [--pdf <annual_report.pdf>] [-o <output.json>]");
        process::exit(1);
    }

    ExtractArgs {
        ticker,
        year,
        file,
        pdf,
        output,
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
    let mut input = None;
    let mut max_listing_date = None;
    let mut exclude_boards = Vec::new();
    let mut output = None;
    let mut format = "list".to_string();

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-i" | "--input" => {
                if let Some(v) = iter.next() {
                    input = Some(PathBuf::from(v));
                }
            }
            "--max-listing-date" | "--max-date" => {
                if let Some(v) = iter.next() {
                    max_listing_date = Some(v.clone());
                }
            }
            "--exclude-board" | "--exclude-boards" => {
                if let Some(v) = iter.next() {
                    for b in v.split(',') {
                        let trimmed = b.trim();
                        if !trimmed.is_empty() {
                            exclude_boards.push(trimmed.to_string());
                        }
                    }
                }
            }
            "--format" => {
                if let Some(v) = iter.next() {
                    format = v.to_lowercase();
                }
            }
            "-o" | "--output" => {
                if let Some(v) = iter.next() {
                    output = Some(PathBuf::from(v));
                }
            }
            "-h" | "--help" => {
                eprintln!("usage: idx-prism sector [-i <securities.json>] [--max-listing-date <YYYY-MM-DD>] [--exclude-board <Board1,Board2>] [--format list|csv|json] [-o <output_file>]");
                process::exit(0);
            }
            _ => {}
        }
    }

    SectorArgs {
        input,
        max_listing_date,
        exclude_boards,
        output,
        format,
    }
}

fn run_sector(raw_args: &[String]) -> Result<(), Box<dyn Error>> {
    let args = parse_sector_args(raw_args);

    let input_path = if let Some(ref p) = args.input {
        p.clone()
    } else if let Ok(dir) = env::var("IDXLENS_DATA") {
        let p = PathBuf::from(dir).join("idx_properties_securities.json");
        if p.exists() {
            p
        } else {
            resolve_default_securities_path()?
        }
    } else {
        resolve_default_securities_path()?
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
    let mut input = None;
    let mut ticker = None;
    let mut year = None;
    let mut format = "csv".to_string();
    let mut output = None;
    let mut shares_csv = None;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-i" | "--input" => {
                if let Some(v) = iter.next() {
                    input = Some(PathBuf::from(v));
                }
            }
            "-t" | "--ticker" => {
                if let Some(v) = iter.next() {
                    ticker = Some(v.to_uppercase());
                }
            }
            "-y" | "--year" => {
                if let Some(v) = iter.next() {
                    year = v.parse::<u32>().ok();
                }
            }
            "--format" => {
                if let Some(v) = iter.next() {
                    format = v.to_lowercase();
                }
            }
            "-o" | "--output" => {
                if let Some(v) = iter.next() {
                    output = Some(PathBuf::from(v));
                }
            }
            "--shares-csv" => {
                if let Some(v) = iter.next() {
                    shares_csv = Some(PathBuf::from(v));
                }
            }
            "-h" | "--help" => {
                eprintln!("usage: idx-prism market -i <file.json|file.csv|dir> [-t <TICKER>] [-y <YEAR>] [--format csv|json] [-o <output_file>] [--shares-csv <ticker,year,shares>]");
                process::exit(0);
            }
            _ => {}
        }
    }

    let input = input.ok_or("Error: -i / --input <file|dir> is required")?;

    Ok(MarketArgs {
        input,
        ticker,
        year,
        format,
        output,
        shares_csv,
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
    let turnover = shares_outstanding.filter(|s| *s > 0.0).map(|so| {
        let vol_sum: u64 = bars.iter().map(|b| b.volume).sum();
        (vol_sum as f64 / bars.len() as f64) / so
    });

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
        pub ownership: Ownership,
    }

    #[derive(Debug, Default)]
    pub struct Ownership {
        pub public_shares: Option<i64>,
        pub total_shares: Option<i64>,
        pub free_float_pct: Option<f64>,
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
    // Layout yang pecah (nama & angka terpisah baris, mis. CTRA/SMRA) -> None
    // (dikoding manual), supaya tidak pernah menghasilkan angka salah senyap.
    pub(crate) fn extract_ownership(text: &str) -> Ownership {
        // jumlah saham: integer besar ber-titik; persen: desimal koma (gaya Indonesia)
        let re_share = Regex::new(r"(\d{1,3}(?:\.\d{3})+)").unwrap();
        let re_pct = Regex::new(r"(\d{1,3},\d+)\s*%?").unwrap();
        let re_public = Regex::new(r"(?i)masyarakat|\bpublic\b").unwrap();
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

        // cari baris publik INLINE pertama (nama + saham + persen dalam satu baris).
        // scan global (bukan anchor note) — sebutan "capital stock" muncul di narasi
        // jauh sebelum tabel Modal Saham sebenarnya.
        let inline_row = |l: &str| -> bool { first_share(l).is_some() && first_pct(l).is_some() };

        let pub_idx = lines
            .iter()
            .position(|l| re_public.is_match(l) && inline_row(l));
        let pub_idx = match pub_idx {
            Some(i) => i,
            None => return Ownership::default(),
        };

        // cari baris Jumlah/Total INLINE terdekat setelahnya (<=40 baris)
        let total_idx = lines[(pub_idx + 1)..(pub_idx + 41).min(n)]
            .iter()
            .position(|l| re_total.is_match(l) && inline_row(l))
            .map(|k| pub_idx + 1 + k);
        let total_idx = match total_idx {
            Some(i) => i,
            None => return Ownership::default(),
        };

        let public_shares = first_share(lines[pub_idx]);
        let free_float = first_pct(lines[pub_idx]);
        let total_shares = first_share(lines[total_idx]);
        let total_pct = first_pct(lines[total_idx]);

        let mut own = Ownership::default();
        if let (Some(ps), Some(ff), Some(ts), Some(tp)) =
            (public_shares, free_float, total_shares, total_pct)
        {
            // total row harus ~100%; cross-check rasio publik == free_float
            let total_ok = (99.5..=100.05).contains(&tp);
            let xcheck_ok = ts > 0 && (100.0 * ps as f64 / ts as f64 - ff).abs() <= 0.5;
            if total_ok && xcheck_ok {
                own.free_float_pct = Some((ff * 100.0).round() / 100.0);
                own.public_shares = Some(ps);
                own.total_shares = Some(ts);
            }
        }
        own
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
        assert_eq!(own.free_float_pct, Some(31.30));
        assert_eq!(own.public_shares, Some(15_070_264_960));
        assert_eq!(own.total_shares, Some(48_159_602_400));
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
        let own = super::pdf_extract::extract_ownership(txt);
        assert_eq!(own.free_float_pct, None);
        assert_eq!(own.total_shares, None);
    }

    #[test]
    fn test_extract_ownership_rejects_bad_xcheck() {
        // baris publik inline ADA, tapi rasio public/total tidak cocok dgn persen
        // -> cross-check gagal -> None (bunuh false-positive)
        let txt = "MODAL SAHAM\n\
Masyarakat 5.000.000.000 50,00 100.000.000\n\
Jumlah 48.000.000.000 100,00 900.000.000\n";
        let own = super::pdf_extract::extract_ownership(txt);
        // 5e9/48e9 = 10.4% != 50% -> reject
        assert_eq!(own.free_float_pct, None);
    }
}
