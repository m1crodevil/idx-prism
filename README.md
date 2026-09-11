# idxlens-rust

Local Rust port of [IDXLens](https://github.com/lugassawan/idxlens) focused on extracting investment-property data from Indonesian IDX (Bursa Efek Indonesia) XBRL financial reports.

## What it does

Given a local IDX `instance.zip` XBRL archive, this CLI extracts:

- `current_year_instant`: current-year carrying value of investment properties.
- `prior_year_instant`: prior-year carrying value.
- `policy_text`: the accounting policy narrative from the `InvestmentPropertiesTextBlock` tag.
- `accounting_model`: whether the issuer uses the **cost model** or **fair value model**.

Output is JSON to stdout or a file.

## Build

```bash
cargo build --release
```

The binary will be at `./target/release/idxlens_rust`.

## Usage

```bash
idxlens_rust <TICKER> -f <path/to/instance.zip> [-y <year>] [-o output.json]
```

Example:

```bash
./target/release/idxlens_rust CTRA -f /path/to/CTRA_2024_instance.zip
```

Sample output:

```json
{
  "ticker": "CTRA",
  "year": 2024,
  "current_year_instant": 4996056000000,
  "prior_year_instant": 5189234000000,
  "policy_text": "Properti investasi adalah properti ...",
  "accounting_model": "cost model"
}
```

## How it works

1. **Read the archive**: open the ZIP and locate the `.xbrl` instance file.
2. **Numeric facts**: use `quick-xml` to stream events and pick up `idx-cor:InvestmentProperties` facts. The `contextRef` attribute tells us whether the value belongs to the current or prior year instant.
3. **Policy text**: because `quick-xml` skips some text-block tags in large XBRL documents, we fall back to a regex that captures `<idx-cor:InvestmentPropertiesTextBlock ...>...</idx-cor:InvestmentPropertiesTextBlock>`. We try the plural form first, then the singular form.
4. **Model detection**: the policy text is lower-cased and scanned for Indonesian keywords:
   - `model nilai wajar` → `fair value model`
   - `biaya` → `cost model`
   - otherwise `unknown`

## Why regex fallback?

The original Go `idxlens` and quick streaming parsers can miss the plural `InvestmentPropertiesTextBlock` tag in some IDX XBRL files. The tag is present in the raw XML and visible to Python/lxml, but `quick-xml::Reader` stops emitting the event in the full document. A targeted regex on the predictable tag is the smallest, reliable workaround.

## Limitations

- This tool only accepts **local** XBRL archives. It does not download from IDX/Cloudflare.
- Detection is based on Indonesian policy text keywords. English or bilingual policies may need keyword expansion.
- XBRL tag namespace prefix is assumed to be `idx-cor`. A different prefix would require a small regex adjustment.

## Dependencies

- `quick-xml` — streaming XML parsing.
- `regex` — robust text-block extraction fallback.
- `serde_json` — JSON output.
- `zip` — archive reading.

## License

MIT
