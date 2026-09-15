# idxlens-rust

Local Rust CLI for extracting investment-property disclosure and financial control variables from Indonesian Stock Exchange (IDX / BEI) XBRL instance packages and PDF Annual Reports / CALK.

Built for empirical accounting research on investment property disclosures (PSAK 13 / IAS 40).

## Capabilities

Given a local IDX `instance.zip` (and optional Annual Report / CALK PDF):

1. **Investment Property Facts (XBRL)**:
   - `current_year_instant`: Carrying value of investment properties (end of current period).
   - `prior_year_instant`: Prior-year carrying value.
   - `accounting_model`: Classified as `cost model` or `fair value model` (from XBRL text block or PDF accounting policy fallback).
   - `policy_text`: Extracted accounting policy narrative (with tag-stripping and fallback handling).
2. **Control Variables (XBRL)**:
   - `total_assets` (`Assets`, current instant)
   - `total_liabilities` (`Liabilities`, current instant)
   - `equity` (`Equity`, current instant)
   - `revenues` (`SalesAndRevenue` / `Revenues`, current duration)
   - `net_income` (`ProfitLoss`, current duration)
3. **Fair Value & Disclosure Disclosures (PDF / CALK)**:
   - `pdf_fair_value_amount`: Verbatim sentence disclosing fair value of investment property under cost model.
   - `pdf_appraiser_name`: Independent appraiser (KJPP) names.
   - `pdf_appraisal_date`: Valuation report date.
   - `pdf_property_location_composition`: Property locations and composition.

Output is formatted as structured JSON.

## Build

```bash
cargo build --release
```

Binary: `./target/release/idxlens_rust`

## Usage

### 1. Extract Financial & Disclosure Data

```bash
idxlens_rust <TICKER> -f <path/to/instance.zip> -y <year> [--pdf <path/to/report.pdf>] [-o <output.json>]
```

**Examples:**

*Full extraction (XBRL + PDF CALK):*
```bash
./target/release/idxlens_rust CTRA \
  -f ~/.idxlens/data/CTRA/2023/Audit/instance.zip \
  -y 2023 \
  --pdf "~/.idxlens/data/CTRA/2023/Audit/PT Ciputra Development Tbk 31 Desember 2023.pdf" \
  -o /tmp/ctra_2023.json
```

*XBRL facts only (fast, no PDF):*
```bash
./target/release/idxlens_rust CTRA \
  -f ~/.idxlens/data/CTRA/2023/Audit/instance.zip \
  -y 2023
```

### 2. Sector Listing & Emiten Details

Extract all listed issuers in the sector with metadata (`Code, Name, ListingDate, IPO_Year, ListingBoard, Shares`) for review or manual filtering:

```bash
idxlens_rust sector [OPTIONS]
```

**Options:**
- `-i, --input <FILE>`: Path to securities JSON (defaults to `~/.idxlens/data/idx_properties_securities.json`).
- `--format <csv|list|json>`: Output format (default: `list`).
- `-o, --output <FILE>`: Save output to file.
- `--max-listing-date <YYYY-MM-DD>`: Optional filter by listing date.
- `--exclude-board <BOARD1,BOARD2>`: Optional board exclusion.

**Examples:**

*Export all 93 sector issuers with full details to CSV for manual examination:*
```bash
./target/release/idxlens_rust sector --format csv -o emiten_sektor_properti.csv
```

*Get comma-separated ticker list for batch downloads:*
```bash
./target/release/idxlens_rust sector
```

### 3. Market Microstructure & Asymmetry Proxies (Tahap D)

Calculate annual information asymmetry proxies and liquidity metrics from daily trading data (Yahoo Finance chart JSON or daily OHLCV CSV):

```bash
idxlens_rust market -i <path/to/data.json|csv|dir> [OPTIONS]
```

**Options:**
- `-i, --input <FILE|DIR>`: Path to daily Yahoo chart JSON, OHLCV CSV, or directory of files (required).
- `-t, --ticker <TICKER>`: Override ticker symbol (optional).
- `-y, --year <YEAR>`: Filter by year (optional).
- `--format <csv|json>`: Output format (default: `csv`).
- `-o, --output <FILE>`: Save metrics to file (default: stdout).

**Metrics Calculated:**
1. **Simple Relative Spread**: Mean of daily $\frac{2(\text{High} - \text{Low})}{\text{High} + \text{Low}}$.
2. **Corwin & Schultz (2012) Spread**: Gold standard 2-day high-low bid-ask spread estimator.
3. **Amihud (2002) Illiquidity**: Mean of $\frac{|\text{Return}_t|}{\text{Price}_t \times \text{Volume}_t}$.
4. **Annual Volume**: Total shares traded in the year.

**Example:**
```bash
./target/release/idxlens_rust market -i /path/to/PWON.json -o pwon_market_2023.csv
```

### Sample Output

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
  "pdf_appraiser_name": "independent appraisers, KJPP Willson & Rekan, KJPP Rengganis, Hamid & Rekan and KJPP Susan Widjojo & Rekan, in their reports with the latest report dated March 28, 2024",
  "pdf_appraisal_date": "laporan terakhir tanggal 28 Maret 2024",
  "pdf_property_location_composition": "Properti investasi terutama merupakan tanah, bangunan pusat niaga dan kawasan komersial, dan ruang kantor yang terletak di Jakarta, Tangerang, Semarang, dan Surabaya."
}
```

## How It Works

1. **XBRL Fact Stream**: Streams XML events via `quick-xml`. Extracts numeric elements (`InvestmentProperties`, `Assets`, `Liabilities`, `Equity`, `SalesAndRevenue`, `ProfitLoss`) matching current-period context references (`CurrentYearInstant`, `CurrentYearDuration`).
2. **Text Block Extraction & Fallback**:
   - Matches `<idx-cor:InvestmentPropertiesTextBlock>` via regex fallback (handles quick-xml plural tag stream limitations in large XML instances).
   - If XBRL text block contains placeholder pointers (e.g. `"Idem row 10"`), falls back automatically to the extracted accounting policy paragraph in the PDF CALK.
3. **CALK Targeted Scrape**:
   - Uses `pdf_oxide` to extract text.
   - Anchors regex extraction to the investment property disclosure note (Note 14 / Note 13) to avoid false positives from Property, Plant & Equipment (PPE) notes.

## Dependencies

- `quick-xml`: Fast streaming XML parser.
- `pdf_oxide`: Pure Rust PDF text extractor.
- `regex`: Targeted block and disclosure sentence pattern matching.
- `serde` / `serde_json`: Serialization.
- `zip`: In-memory extraction of compressed XBRL instance files.

## License

MIT
