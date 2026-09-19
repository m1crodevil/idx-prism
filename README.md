<div align="center">

# IDX-Prism

**High-Performance Pipeline for Financial Disclosure Extraction & Market Microstructure Analytics on the Indonesia Stock Exchange (IDX / BEI)**

[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)]()
[![Tests](https://img.shields.io/badge/tests-passing-brightgreen.svg)]()
[![IDX Taxonomy](https://img.shields.io/badge/taxonomy-IDX%202020%2B-informational.svg)](https://www.idx.co.id)
[![Accounting Standards](https://img.shields.io/badge/standards-PSAK%2013%20%7C%20240%20%7C%20IAS%2040-success.svg)]()

<p align="center">
  <a href="#overview">Overview</a> •
  <a href="#pipeline-architecture">Architecture</a> •
  <a href="#core-capabilities">Capabilities</a> •
  <a href="#installation">Installation</a> •
  <a href="#cli-usage">CLI Usage</a> •
  <a href="#methodology--citation">Citation</a>
</p>

</div>

---

## Overview

**IDX-Prism** is a fast, memory-safe Rust toolkit engineered for empirical capital market and accounting research on the **Indonesia Stock Exchange (Bursa Efek Indonesia / BEI)**.

Analyzing Indonesian public firms typically requires labor-intensive manual retrieval across disconnected formats: XML/XBRL taxonomy instances, scanned or unstructured Annual Report PDF disclosures (Catatan atas Laporan Keuangan / CALK), and historical market order book / trade logs.

IDX-Prism consolidates these workflows into a single autonomous engine:
1. **XBRL Fact Streaming**: Parses financial statement elements and firm-level control variables without memory bloat.
2. **CALK Disclosure Extraction**: Context-aware regex parser anchored to specific disclosure notes (e.g. investment properties under PSAK 13 / IAS 40), extracting verbatim fair value amounts, independent appraisal firms (KJPP), appraisal dates, and asset compositions. Bilingual filings print the Indonesian and English halves of a sentence on the *same* visual row, so the row merge can carry a firm name across a column boundary (`KJPP independen KJPP Willson & Rekan`). A firm name is `KJPP <people> & Rekan` and never contains the acronym twice, so the match is trimmed to its **last** `KJPP` — that yields the true name instead of the English prose bleeding in. Rust's `regex` crate has no look-around by design, so this filter runs in code rather than in the pattern. Names are cut to the investment-property note only: an appraiser quoted for an acquisition or a business combination must not be reported as the investment-property appraiser. A second, independent reading of the same field can be requested from a vision model (`--vlm`), which reports its own answer and whether it agrees.
3. **Market Microstructure & Information Asymmetry**: Computes peer-reviewed econometric proxies (**Corwin-Schultz 2012 spread**, **Zero-Return-Days**, **volatility**, **turnover**) directly from daily OHLCV series.
4. **Share Ownership Extraction**: Parses the CALK *Modal Saham* note to recover public/free-float shares, percentage, and shares outstanding. Page text is rebuilt as visual rows (grouped from word bounding boxes) so bilingual, multi-column tables read as a human sees them, then two gates must pass — **row identity** (the row's own label must be the public holder; management and Total rows are excluded) and **self-consistency** (`public/total == free-float%`). Failing either returns `null` rather than a silently wrong number.
5. **Universe & Sampling Tools**: Queries and exports official IDX-IC sector listings with purposive filtering options.

---

## Pipeline Architecture

```text
[ IDX Financial Data Sources ]
  ├── XBRL instance.zip ─────────► [ idx-prism extract ] ──► Carrying Value, Total Assets,
  │                                                         Liabilities, Equity, Revenue, Net Income
  ├── Annual Report PDF (CALK) ──► [ Note-Anchored NLP ] ──► Fair Value Amount, KJPP Appraiser,
  │                                                         Report Date, Property Composition,
  │                                                         Free-Float / Shares Outstanding
  └── Daily OHLCV (Yahoo/IDX) ──► [ idx-prism market  ] ──► Corwin-Schultz (2012) Spread,
                                                            Amihud (2002) Illiquidity, ZRD,
                                                            Volatility, Turnover
                                             │
                                             ▼
                             [ Balanced Empirical Panel Dataset ]
                               (Ready for Stata / R / Python OLS & Panel Regression)
```

---

## Core Capabilities

* **Deterministic XBRL Streaming**: Zero-allocation byte-slice tag matching (`quick-xml`) across massive financial position and profit/loss XBRL instances.
* **Intelligent Accounting Policy Resolution**: Reads the first non-empty `InvestmentProperty(ies)TextBlock` in document order, so the substantive policy wins over a cross-reference placeholder (some issuers emit only `"Idem row 10"` in one block while the real narrative sits in the other). When every XBRL block is empty or `nil`, it falls back to the policy narrative from the audited PDF CALK text layer.
* **Standardized Control Variables**: Automatically maps `Assets`, `Liabilities`, `Equity`, `SalesAndRevenue`, and `ProfitLoss` to current-period instants and durations.
* **Econometric Spread Estimation**: Computes the gold-standard Corwin & Schultz (2012) two-day high-low bid-ask spread estimator, plus Zero-Return-Days and daily-return volatility, to quantify information asymmetry without requiring proprietary intraday tick data.
* **Share-Ownership Extraction**: Parses the CALK *Modal Saham* note to recover public/free-float shares, percentage, and shares outstanding. A self-consistency gate alone is insufficient — every row satisfies `public/total == printed percent`, including a director's line — so it is paired with a row-identity gate. Either gate failing yields `null`.
* **Page Provenance and Loud Unreadable-PDF Failure**: Each PDF-derived field records the 1-based page it was anchored to (`pdf_ip_region_page`), so any value can be re-opened and confirmed at its source. A PDF with no extractable text layer (scanned or image-only) fails with an explicit error instead of returning empty fields — otherwise "could not be read" would be indistinguishable from "the note is absent" and would silently shrink the sample.
* **Sanitized & Portable**: Zero hardcoded local machine paths; configurable via environment variables (`IDXPRISM_DATA`, `IDXPRISM_BIN`).

---

## Installation

### Prerequisites
* [Rust Toolchain](https://www.rust-lang.org/tools/install) (version 1.75 or later)

### Build from Source
```bash
git clone https://github.com/m1crodevil/idx-prism.git
cd idx-prism
cargo build --release                 # deterministic pipeline (no network, no credentials)
cargo build --release --features vlm  # + the vision validation channel (see below)
```

The compiled binary will be available at `./target/release/idx-prism`.

### Verify the Build
```bash
cargo test                        # unit tests (includes a sanitization gate)
cargo test --features vlm         # + the vision channel's own tests
./scripts/smoke_test.sh           # end-to-end exercise of every subcommand on local data
```
`smoke_test.sh` reads issuer data from `$IDXPRISM_DATA` (default `~/.idxlens/data`) and skips emiten whose files are absent.

---

## CLI Usage

### 1. Extract Financial & CALK Disclosures (`extract`)

Extracts investment property accounting models, carrying amounts, control variables, and fair value notes:

```bash
idx-prism <TICKER> -f <path/to/instance.zip> -y <year> [--pdf <path/to/report.pdf>] [-o <output.json>]
```

**Example:**
```bash
./target/release/idx-prism CTRA \
  -f ~/.idxlens/data/CTRA/2023/Audit/instance.zip \
  -y 2023 \
  --pdf "~/.idxlens/data/CTRA/2023/Audit/PT Ciputra Development Tbk 31 Desember 2023.pdf" \
  -o /tmp/ctra_2023.json
```

**Structured JSON Output:**
```json
{
  "ticker": "CTRA",
  "year": 2023,
  "accounting_model": "cost model",
  "policy_text": "Properti investasi adalah properti (tanah atau bangunan atau bagian dari suatu bangunan atau kedua-duanya) untuk menghasilkan sewa atau untuk kenaikan nilai atau keduanya. Properti investasi diukur sebesar biaya perolehan setelah dikurangi akumulasi penyusutan dan akumulasi kerugian penurunan nilai. …  [truncated — the verbatim XBRL policy block, ~2.2k chars]",
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

> **Share ownership** (`free_float_pct`, `public_shares`, `shares_outstanding`) is added to the JSON when the CALK *Modal Saham* table passes both gates. Verified end-to-end on six issuers: PWON 2023 `31.30`, BSDE 2023 `28.94`, **CTRA 2023 `46.60`**, SMRA 2023 `64.54`, APLN 2024 `12.26`, DILD 2024 `36.55`. Labels differ per issuer ("Masyarakat", "Masyarakat umum", "Lain-lain (masing-masing dengan pemilikan kurang dari 5%)", "Public"), so the parser anchors on where the label sits within its row, not on a fixed phrase.
>
> **Choosing the PDF matters more than choosing the parser.** A single filing ships several PDFs, and the one that *mentions* the note most often is not the one that parses — for APLN 2024 the 8.5 MB annual report contains 89 occurrences of "masyarakat" and yields nothing, while the 863 KB audited-statements PDF contains 3 and yields the correct `12.26`. Select by *outcome*: try each candidate and keep the one that produces a value, then use agreement between documents as verification. Measured on 73 emiten-years: 53 (73%) yield a value from at least one document, 50 of those agree across documents, and 3 disagree — one materially (BSDE 2023: `28.94` vs `33.91`, a different reporting period in the same filing), two by rounding (`68.93` vs `68.94`; `2.20` vs `2.25`). For an authoritative figure, the monthly **LBRPE** ("Laporan Bulanan Registrasi Pemegang Efek", field *"% Saham Free Float"*) reflects the official BEI Regulation I-A definition (excluding treasury, lock-up, controllers, and directors) and can validate this proxy.

---

### 2. Sector Universe & Purposive Sampling (`sector`)

Inspects the official IDX-IC sector list and exports issuer metadata (`Code, Name, ListingDate, IPO_Year, ListingBoard, Shares`):

```bash
idx-prism sector [OPTIONS]
```

**Key Options:**
* `--format <csv|list|json>`: Output format (default: comma-separated `list`).
* `--max-listing-date <YYYY-MM-DD>`: Filter by IPO listing date (e.g. `2021-01-01` for balanced multi-year panel).
* `--exclude-board <BOARD1,BOARD2>`: Exclude specific boards (e.g. `Akselerasi,Pemantauan Khusus`).
* `-o, --output <FILE>`: Write results to file.

**Examples:**
```bash
# Export all 93 property & real estate issuers to CSV for exploratory analysis:
./target/release/idx-prism sector --format csv -o emiten_sektor_properti.csv

# Apply purposive sampling criteria (IPO <= 2021-01-01, excluding non-standard boards):
./target/release/idx-prism sector \
  --max-listing-date 2021-01-01 \
  --exclude-board "Akselerasi,Pemantauan Khusus" \
  --format csv \
  -o sample_properti_purposive.csv
```

---

### 3. Market Asymmetry & Liquidity Metrics (`market`)

Processes daily stock trading logs (Yahoo Finance chart JSON or daily OHLCV CSV) into annual liquidity and information asymmetry proxies:

```bash
idx-prism market -i <path/to/data.json|csv|dir> [OPTIONS]
```

**Formulations Computed:**
* **Simple High-Low Spread**: $\text{Mean}\left(\frac{2(\text{High}_t - \text{Low}_t)}{\text{High}_t + \text{Low}_t}\right)$
* **Corwin & Schultz (2012) Spread**: High-Low volatility-adjusted 2-day spread estimator (negative estimates clipped to zero per the original convention).
* **Amihud (2002) Illiquidity**: $\text{Mean}\left(\frac{|\text{Return}_t|}{\text{Price}_t \times \text{Volume}_t}\right)$
* **Zero-Return-Days (ZRD)**: fraction of trading days with a zero daily return (Vergauwe & Gaeremynck, 2019).
* **Volatility**: standard deviation of daily returns (take the natural log downstream in EViews/Stata).
* **Turnover**: mean daily (shares traded ÷ shares outstanding). Requires `--shares-csv`; left empty otherwise — never silently zero.

**Key Options:**
* `-y, --year <YEAR>`: Restrict to one year.
* `--format <csv|json>`: Output format (default: `csv`).
* `--shares-csv <FILE>`: CSV of `ticker,year,shares` (e.g. exported from `extract`'s `shares_outstanding`) to enable the TURNOVER column.
* `-o, --output <FILE>`: Write results to file.

**Example:**
```bash
# ZRD + volatility computed from OHLCV alone:
./target/release/idx-prism market -i /tmp/pwon_yahoo.json -y 2023 --format csv

# add TURNOVER by supplying shares outstanding (ticker,year,shares):
printf "ticker,year,shares\nPWON,2023,48159602400\n" > shares.csv
./target/release/idx-prism market -i /tmp/pwon_yahoo.json -y 2023 --shares-csv shares.csv
```

**Output CSV Format:**
```csv
ticker,year,trading_days,simple_spread,corwin_schultz_spread,amihud_illiquidity,zero_return_days,volatility,turnover,annual_volume
PWON,2023,239,0.024608,0.007184,1.045100e-12,0.113445,0.015110,0.000637,7337606000
```

---

### 4. Automated Batch Processing

Consolidate every downloaded report into a unified panel CSV:

```bash
idx-prism batch -d ~/.idxlens/data --years 2021-2024 -o dataset_properti_panel.csv
```

The PDF for each filing is chosen **by outcome, not by filename**: every PDF beside the filing is tried and the one that fills the most fields wins, with the winner recorded in `pdf_file`. Filename rules picked a document that yielded nothing for 4 of 72 emiten-years while a sibling PDF held the values. When two documents disagree on the free-float percentage the row is flagged `review_required=1` rather than silently keeping one — the BSDE 2023 filing ships two PDFs for different reporting periods (`28.94` vs `33.91`).

Filings that cannot be extracted are reported on stderr and the process exits non-zero, so a partial run cannot pass for a complete one.

**Output columns:** ticker, year, accounting model, investment property amounts, control variables (assets/liabilities/equity/revenue/net income), fair value disclosure flag, appraiser names, free-float ownership proxy (`free_float_pct`, `public_shares`, `shares_outstanding`), the source document (`pdf_file`), and a disagreement flag (`review_required`). Ownership fields are empty when either gate fails.

**`accounting_model` values:** `cost model`, `fair value model`, `revaluation model`, or `unknown`. Classification reads the *measurement sentence* — the one stating how the property is measured **after initial recognition** — rather than mere keyword presence, because every fair-value issuer also writes "at cost on initial recognition", and a keyword match misclassifies them as cost model.

### 5. Vision Validation Channel (`--vlm`)

An opt-in **second reading** of the appraiser field by a vision model looking at the rendered page. It exists to satisfy one research requirement: a reported value must be checkable against something that did not produce it. It never overwrites the deterministic fields — it agrees or disagrees with them.

```bash
cargo build --release --features vlm
export IDXPRISM_VLM_BASE_URL="https://<your-openai-compatible-host>/v1"
export IDXPRISM_VLM_API_KEY="..."          # read from the environment, never stored
export IDXPRISM_VLM_MODEL="ali/kimi-k2.7-code"   # optional, this is the default

idx-prism CTRA -f instance.zip -y 2024 --pdf lapkeu.pdf --vlm
```

Three design choices worth knowing:

* **A window of pages is rendered, not one.** The page provenance (`pdf_ip_region_page`) is a heuristic: measured against per-page ground truth it hit the page exactly for some filings and was off by one for others, because the note anchor and the appraiser sentence do not always share a page. The rendered window is `page-1 .. page+1`, so one wrong index cannot decide the result.
* **The model's NAME is verified against the page's own text layer, not its quote.** Filings are bilingual and the row rebuild interleaves the Indonesian and English halves of a sentence by x-position, so a quote copied from *one* column is by construction not a contiguous span of our page text — requiring a verbatim quote match failed on every firm while the model had read the page correctly. The name is the actual claim, and an invented one still fails, because it occurs nowhere on the page. Each firm is reported with `verified: true|false`.
* **`scope` is reported, not assumed.** The model distinguishes an appraiser used for investment property from one quoted for an acquisition, a business combination, or fixed assets. That distinction matters: an issuer can name a different firm for each.

The channel costs one request per filing, so it is intended for the subset that needs corroboration, not the whole corpus. It is behind a Cargo feature because it pulls a rasteriser and a TLS stack that the deterministic pipeline does not need.

---

## Methodology & Citation

If you use **IDX-Prism** in your academic research, thesis (Karya Akhir / Skripsi / Tesis), or scientific publications (SINTA / Scopus), please cite as:

```bibtex
@software{idx_prism_2026,
  author = {microdevil},
  title = {IDX-Prism: Automated Financial Disclosure & Market Microstructure Pipeline for Indonesian Capital Market Research},
  year = {2026},
  url = {https://github.com/m1crodevil/idx-prism}
}
```

### Key Methodological References
* **Corwin, S. A., & Schultz, P. (2012).** A simple way to estimate bid-ask spreads from daily high and low prices. *The Journal of Finance*, 67(2), 719-760.
* **Amihud, Y. (2002).** Illiquidity and stock returns: cross-section and time-series effects. *Journal of Financial Markets*, 5(1), 31-56.
* **Vergauwe, S., & Gaeremynck, A. (2019).** Do measurement-related fair value disclosures affect information asymmetry? *Accounting and Business Research*, 49(2), 121-152.

---

## License

This project is licensed under the [MIT License](LICENSE).
