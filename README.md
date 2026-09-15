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
2. **CALK Disclosure Extraction**: Context-aware regex parser anchored to specific disclosure notes (e.g. investment properties under PSAK 13 / IAS 40), extracting verbatim fair value amounts, independent appraisal firms (KJPP), appraisal dates, and asset compositions.
3. **Market Microstructure & Information Asymmetry**: Computes peer-reviewed econometric proxies (Corwin-Schultz 2012 bid-ask spreads, Amihud 2002 illiquidity, and high-low spreads) directly from daily OHLCV series.
4. **Universe & Sampling Tools**: Queries and exports official IDX-IC sector listings with purposive filtering options.

---

## Pipeline Architecture

```text
[ IDX Financial Data Sources ]
  ├── XBRL instance.zip ─────────► [ idx-prism extract ] ──► Carrying Value, Total Assets,
  │                                                         Liabilities, Equity, Revenue, Net Income
  ├── Annual Report PDF (CALK) ──► [ Note-Anchored NLP ] ──► Fair Value Amount, KJPP Appraiser,
  │                                                         Report Date, Property Composition
  └── Daily OHLCV (Yahoo/IDX) ──► [ idx-prism market  ] ──► Corwin-Schultz (2012) Spread,
                                                            Amihud (2002) Illiquidity, Volume
                                             │
                                             ▼
                             [ Balanced Empirical Panel Dataset ]
                               (Ready for Stata / R / Python OLS & Panel Regression)
```

---

## Core Capabilities

* **Deterministic XBRL Streaming**: Zero-allocation byte-slice tag matching (`quick-xml`) across massive financial position and profit/loss XBRL instances.
* **Intelligent Accounting Policy Fallback**: When XBRL disclosure blocks contain placeholder pointers (`"Idem row 10"`), the engine transparently resolves the policy narrative from the audited PDF CALK text layer.
* **Standardized Control Variables**: Automatically maps `Assets`, `Liabilities`, `Equity`, `SalesAndRevenue`, and `ProfitLoss` to current-period instants and durations.
* **Econometric Spread Estimation**: Computes the gold-standard Corwin & Schultz (2012) two-day high-low bid-ask spread estimator to quantify information asymmetry without requiring proprietary intraday tick data.
* **Sanitized & Portable**: Zero hardcoded local machine paths; configurable via environment variables (`IDXPRISM_DATA`, `IDXPRISM_BIN`).

---

## Installation

### Prerequisites
* [Rust Toolchain](https://www.rust-lang.org/tools/install) (version 1.75 or later)

### Build from Source
```bash
git clone https://github.com/m1crodevil/idx-prism.git
cd idx-prism
cargo build --release
```

The compiled binary will be available at `./target/release/idx-prism`.

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
  "policy_text": "Properti investasi adalah properti (tanah atau bangunan atau bagian dari suatu bangunan atau kedua-duanya) untuk menghasilkan sewa atau untuk kenaikan nilai atau keduanya. Properti investasi diukur sebesar biaya perolehan setelah dikurangi akumulasi penyusutan",
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
* **Corwin & Schultz (2012) Spread**: High-Low volatility-adjusted 2-day spread estimator.
* **Amihud (2002) Illiquidity**: $\text{Mean}\left(\frac{|\text{Return}_t|}{\text{Price}_t \times \text{Volume}_t}\right)$

**Example:**
```bash
./target/release/idx-prism market -i /tmp/pwon_yahoo.json --format csv
```

**Output CSV Format:**
```csv
ticker,year,trading_days,simple_spread,corwin_schultz_spread,amihud_illiquidity,annual_volume
PWON,2023,239,0.024608,0.007184,1.045100e-12,7337606000
```

---

### 4. Automated Batch Processing

Consolidate all downloaded reports into a unified panel CSV in seconds:

```bash
./scripts/batch_extract.sh dataset_properti_panel.csv
```

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
