#!/usr/bin/env bash
set -euo pipefail

# Batch extractor untuk idx-prism ke CSV/JSONL
OUT_CSV="${1:-/tmp/dataset_properti_2023.csv}"
DATA_DIR="${IDXPRISM_DATA:-${IDXLENS_DATA:-$HOME/.idxlens/data}}"
BINARY="${IDXPRISM_BIN:-${IDXLENS_BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/release/idx-prism}}"

echo "ticker,year,accounting_model,current_ip,prior_ip,assets,liabilities,equity,revenues,net_income,has_fv_disclosure,appraiser" > "$OUT_CSV"

for ticker_dir in "$DATA_DIR"/*; do
  [ -d "$ticker_dir" ] || continue
  ticker=$(basename "$ticker_dir")
  
  for year_dir in "$ticker_dir"/*; do
    [ -d "$year_dir" ] || continue
    year=$(basename "$year_dir")
    
    # Audit atau Tahunan
    for audit_dir in "$year_dir"/Audit "$year_dir"/FY; do
      [ -d "$audit_dir" ] || continue
      
      instance_zip="$audit_dir/instance.zip"
      [ -f "$instance_zip" ] || continue
      
      # Cari PDF Annual Report atau LK lengkap (ambil PDF non-surat/checklist terbesar)
      pdf_file=""
      max_size=0
      for p in "$audit_dir"/*.pdf; do
        [ -f "$p" ] || continue
        fname=$(basename "$p")
        case "$fname" in
          *Checklist*|*CheckList*|*CHECKLIST*|*SPD*|*Surat*|*att*|*025.*) continue ;;
        esac
        size=$(stat -c%s "$p" 2>/dev/null || stat -f%z "$p" 2>/dev/null || wc -c < "$p")
        if [ "$size" -gt "$max_size" ]; then
          max_size="$size"
          pdf_file="$p"
        fi
      done

      cmd=("$BINARY" "$ticker" -f "$instance_zip" -y "$year")
      if [ -n "$pdf_file" ]; then
        cmd+=(--pdf "$pdf_file")
      fi
      
      json_out=$("${cmd[@]}" 2>/dev/null || echo "")
      if [ -n "$json_out" ]; then
        python3 -c '
import sys, json, csv

data = json.loads(sys.argv[1])
appraiser = data.get("pdf_appraiser_name", "").replace(",", ";")
row = [
    data.get("ticker", ""),
    data.get("year", ""),
    data.get("accounting_model", ""),
    data.get("current_year_instant", ""),
    data.get("prior_year_instant", ""),
    data.get("total_assets", ""),
    data.get("total_liabilities", ""),
    data.get("equity", ""),
    data.get("revenues", ""),
    data.get("net_income", ""),
    "1" if data.get("pdf_fair_value_amount") else "0",
    appraiser
]
print(",".join(str(x) for x in row))
' "$json_out" >> "$OUT_CSV"
      fi
    done
  done
done

echo "Dataset tersimpan di $OUT_CSV"
cat "$OUT_CSV"
