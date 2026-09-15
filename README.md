# IDX-Prism

**IDX-Prism: Automated Financial Disclosure & Market Microstructure Pipeline for Indonesian Capital Market Research**

A high-performance Rust toolkit for parsing Indonesia Stock Exchange (IDX / BEI) XBRL financial reports, extracting CALK disclosure notes from Annual Report PDFs, and computing empirical market microstructure / information asymmetry metrics.

Built for empirical accounting and financial research (e.g. PSAK 13 / IAS 40 investment property disclosures and information asymmetry studies).

## Core Capabilities

1. **Financial Statements & Controls (`extract`)**:
   - Streams XBRL instances to extract carrying amounts (`InvestmentProperties`).
   - Automatically parses financial control variables: `total_assets` (`Assets`), `total_liabilities` (`Liabilities`), `equity` (`Equity`), `revenues` (`SalesAndRevenue`), and `net_income` (`ProfitLoss`).
   - Classifies accounting model (`cost model` vs `fair value model`).
   - Extracts verbatim fair value disclosures, independent appraiser (KJPP) names, appraisal dates, and property compositions from PDF Notes to Consolidated Financial Statements (CALK).
   - Graceful fallback: when XBRL text blocks contain placeholder pointers (`"Idem row 10"`), automatically falls back to the audited PDF CALK accounting policy.

2. **Sector Universe & Sampling (`sector`)**:
   - Parses official IDX-IC sector listings (e.g. Properties & Real Estate).
   - Exports complete issuer metadata (`Code`, `Name`, `ListingDate`, `IPO_Year`, `ListingBoard`, `Shares`) to CSV/JSON for manual analysis and purposive sampling.
   - Built-in purposive filtering options (`--max-listing-date`, `--exclude-board`).

3. **Market Microstructure Metrics (`market`)**:
   - Ingests daily market trading data (Yahoo Finance chart JSON or daily OHLCV CSVs).
   - Computes annual empirical information asymmetry and liquidity metrics:
     * **Simple Relative High-Low Spread**
     * **Corwin & Schultz (2012) Bid-Ask Spread Estimator**
     * **Amihud (2002) Price Impact / Illiquidity Ratio**
     * **Annual Trading Volume**

## Installation & Build

Requires Rust toolchain (1.75+):

```bash
cargo build --release
```

The optimized binary will be produced at `./target/release/idx-prism`.

## CLI Usage

### 1. Extract Financial & CALK Disclosures

```bash
idx-prism <TICKER> -f <path/to/instance.zip> -y <year> [--pdf <path/to/report.pdf>] [-o <output.json>]
```

**Examples:**

*Full extraction (XBRL + PDF CALK):*
```bash
./target/release/idx-prism CTRA \
  -f ~/.idxlens/data/CTRA/2023/Audit/instance.zip \
  -y 2023 \
  --pdf "~/.idxlens/data/CTRA/2023/Audit/PT Ciputra Development Tbk 31 Desember 2023.pdf" \
  -o /tmp/ctra_2023.json
```

*XBRL facts only (fast, without PDF):*
```bash
./target/release/idx-prism CTRA \
  -f ~/.idxlens/data/CTRA/2023/Audit/instance.zip \
  -y 2023
```

**Output Structure:**
```json
{
  "ticker": "CTRA",
  "year": 2023,
  "accounting_model": "cost model",
  "policy_text": "Properti investasi adalah properti (tanah atau bangunan atau bagian dari suatu bangunan atau kedua-duanya) untuk menghasilkan sewa atau untuk kenaikan nilai atau keduanya. Properti investasi diukur sebesar biaya perolehan setelah dikurangi akumulasi penyusutan dan akumulasi kerugian penurunan nilai",
  "current_year_instant": 5189234000000,
  "prior_year_instant": 5349310000000,
  "total_assets": 44115215000000,
  "total_liabilities": 21490499000000,
  "equity": 22624716000000,
  "revenues": 9245032000000,
  "net_income": 1909025000000,
  "pdf_fair_value_amount": "Nilai wajar properti investasi tertentu adalah sebesar Rp13.043.481 yang ditentukan berdasarkan penilaian yang dilakukan oleh penilai independen KJPP Willson & Rekan, KJPP Rengganis, Hamid & Rekan dan KJPP Susan Widjojo & Rekan, dalam laporan-laporannya dengan laporan terakhir tanggal 28 Maret 2024.",
  "pdf_appraiser_name": "KJPP Willson & Rekan, KJPP Rengganis, Hamid & Rekan, KJPP Susan Widjojo & Rekan",
  "pdf_appraisal_date": "laporan terakhir tanggal 28 Maret 2024",
  "pdf_property_location_composition": "Properti investasi terutama merupakan tanah, bangunan pusat niaga dan kawasan komersial, dan ruang kantor yang terletak di Jakarta, Tangerang, Semarang, dan Surabaya."
}
```

---

### 2. Sector Metadata & Export

Extract sector universe metadata for review or manual filtering:

```bash
idx-prism sector [OPTIONS]
```

**Options:**
- `-i, --input <FILE>`: Path to securities JSON (defaults to `~/.idxlens/data/idx_properties_securities.json`).
- `--format <csv|list|json>`: Output format (default: `list`).
- `-o, --output <FILE>`: Save output to file.
- `--max-listing-date <YYYY-MM-DD>`: Optional listing date threshold.
- `--exclude-board <BOARD1,BOARD2>`: Optional board exclusion.

**Examples:**

*Export all 93 property sector issuers to CSV with full details:*
```bash
./target/release/idx-prism sector --format csv -o emiten_sektor_properti.csv
```

*Filter sample listing <= 2021-01-01 and exclude boards:*
```bash
./target/release/idx-prism sector \
  --max-listing-date 2021-01-01 \
  --exclude-board "Akselerasi,Pemantauan Khusus" \
  --format csv \
  -o sample_emiten_properti.csv
```

---

### 3. Market Asymmetry & Liquidity Metrics

Calculate annual information asymmetry proxies and liquidity metrics from daily trading data (Yahoo Finance chart JSON or daily OHLCV CSV):

```bash
idx-prism market -i <path/to/data.json|csv|dir> [OPTIONS]
```

**Options:**
- `-i, --input <FILE|DIR>`: Path to daily Yahoo chart JSON, OHLCV CSV, or directory of files (required).
- `-t, --ticker <TICKER>`: Override ticker symbol (optional).
- `-y, --year <YEAR>`: Filter by year (optional).
- `--format <csv|json>`: Output format (default: `csv`).
- `-o, --output <FILE>`: Save metrics to file (default: stdout).

**Example:**
```bash
./target/release/idx-prism market -i /path/to/PWON.json -o pwon_market_2023.csv
```

**Output CSV Format:**
```csv
ticker,year,trading_days,simple_spread,corwin_schultz_spread,amihud_illiquidity,annual_volume
PWON,2023,239,0.024608,0.007184,1.045100e-12,7337606000
```

---

### 4. Batch Automation Pipeline

To extract all downloaded issuer archives to a consolidated CSV dataset:

```bash
./scripts/batch_extract.sh output_dataset_2023.csv
```

Configurable via environment variables:
- `IDXPRISM_DATA`: Path to downloaded IDX data directory (default: `~/.idxlens/data`).
- `IDXPRISM_BIN`: Path to `idx-prism` executable.

## Technical Architecture

- **XML Streaming**: Zero-allocation byte-slice matching on `quick-xml` reader events for numeric facts.
- **CALK PDF Engine**: Direct text layer extraction via `pdf_oxide` anchored to specific CALK notes (e.g. Note 14, 13, or 10).
- **Mathematical Formulations**:
  - Corwin & Schultz (2012) 2-day high-low bid-ask spread estimator.
  - Amihud (2002) daily absolute return to dollar volume ratio.

## Dependencies

- `quick-xml`: Fast streaming XML parser.
- `pdf_oxide`: Pure Rust PDF text extractor.
- `regex`: Targeted block and disclosure pattern matching.
- `serde` / `serde_json`: Serialization and deserialization.
- `zip`: In-memory archive reader.

## License

MIT
