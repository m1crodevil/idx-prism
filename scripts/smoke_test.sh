#!/usr/bin/env bash
# ponytail: smoke test — thin end-to-end exercise of every subcommand on real
# local data. Asserts exit codes and that required fields appear; not a diff
# harness (that was the refactor golden). Run after a clean build.
set -uo pipefail

BIN="${1:-./target/release/idx-prism}"
DATA="${IDXPRISM_DATA:-$HOME/.idxlens/data}"
pass=0; fail=0

ok()   { echo "  PASS  $1"; pass=$((pass+1)); }
bad()  { echo "  FAIL  $1"; fail=$((fail+1)); }
check(){ # check <label> <exit_code> <expected>
  if [ "$2" -eq "$3" ]; then ok "$1 (exit=$2)"; else bad "$1 (exit=$2, want $3)"; fi
}

echo "== smoke: binary =="
[ -x "$BIN" ] && ok "binary executable" || { bad "binary missing at $BIN"; exit 1; }

echo "== smoke: no-arg / bad-arg paths exit non-zero, no panic =="
out=$("$BIN" 2>&1); check "no args -> usage" $? 1
echo "$out" | grep -qi "panic" && bad "no-args panicked" || ok "no-args no panic"
out=$("$BIN" PWON -f "$DATA/PWON/2023/Audit/instance.zip" 2>&1); check "missing -y" $? 1
echo "$out" | grep -q "year is required" && ok "missing -y names the flag" || bad "missing -y message wrong"

echo "== smoke: --help exits 0 =="
"$BIN" sector -h >/dev/null 2>&1; check "sector -h" $? 0
"$BIN" market -h >/dev/null 2>&1; check "market -h" $? 0

echo "== smoke: sector =="
"$BIN" sector --format csv >/tmp/sm_sector.csv 2>/dev/null; check "sector csv" $? 0
n=$(wc -l </tmp/sm_sector.csv); [ "$n" -gt 50 ] && ok "sector csv rows=$n" || bad "sector csv rows=$n"
head -1 /tmp/sm_sector.csv | grep -q "Code,Name,ListingDate,IPO_Year,ListingBoard,Shares" \
  && ok "sector csv header" || bad "sector csv header"
"$BIN" sector --format json >/dev/null 2>&1; check "sector json" $? 0
"$BIN" sector >/dev/null 2>&1; check "sector list" $? 0
"$BIN" sector --max-listing-date 2021-01-01 --exclude-board Akselerasi --format csv >/tmp/sm_filtered.csv 2>/dev/null
check "sector filtered" $? 0
[ "$(wc -l </tmp/sm_filtered.csv)" -lt "$n" ] && ok "filter reduced universe" || bad "filter did not reduce"

echo "== smoke: extract (per emiten) =="
for t in PWON ASRI BSDE CTRA SMRA; do
  zip="$DATA/$t/2023/Audit/instance.zip"
  [ -f "$zip" ] || { echo "  skip  $t (no local data)"; continue; }
  "$BIN" "$t" -f "$zip" -y 2023 >"/tmp/sm_$t.json" 2>/dev/null
  check "extract $t" $? 0
  grep -q '"accounting_model"' "/tmp/sm_$t.json" && ok "$t has accounting_model" || bad "$t missing accounting_model"
done

echo "== smoke: extract with PDF (CALK notes) =="
"$BIN" PWON -f "$DATA/PWON/2023/Audit/instance.zip" -y 2023 \
  --pdf "$DATA/PWON/2023/Audit/PT Pakuwon Jati Tbk - 31 Desember 2023 (FINAL).pdf" \
  -o /tmp/sm_pwon_pdf.json >/dev/null 2>&1
check "extract PWON --pdf" $? 0
grep -q '"free_float_pct"' /tmp/sm_pwon_pdf.json && ok "PWON free_float extracted" || bad "PWON free_float missing"
grep -q '"shares_outstanding"' /tmp/sm_pwon_pdf.json && ok "PWON shares_outstanding extracted" || bad "PWON shares_outstanding missing"

echo "== smoke: market =="
[ -f /tmp/pwon_yahoo.json ] || { echo "  skip  market (no /tmp/pwon_yahoo.json)"; }
if [ -f /tmp/pwon_yahoo.json ]; then
  "$BIN" market -i /tmp/pwon_yahoo.json -y 2023 >/tmp/sm_market.csv 2>/dev/null; check "market csv" $? 0
  grep -q "^ticker,year,trading_days" /tmp/sm_market.csv && ok "market header" || bad "market header"
  # turnover empty without --shares-csv
  awk -F, 'NR==2 && $9==""' /tmp/sm_market.csv >/dev/null && ok "turnover empty w/o shares" || bad "turnover not empty w/o shares"
  printf "ticker,year,shares\nPWON,2023,48159602400\n" >/tmp/sm_shares.csv
  "$BIN" market -i /tmp/pwon_yahoo.json -y 2023 --shares-csv /tmp/sm_shares.csv >/tmp/sm_turn.csv 2>/dev/null
  check "market --shares-csv" $? 0
  awk -F, 'NR==2 && $9!=""' /tmp/sm_turn.csv >/dev/null && ok "turnover populated w/ shares" || bad "turnover not populated"
  "$BIN" market -i /tmp/pwon_yahoo.json --format json >/dev/null 2>&1; check "market json" $? 0
fi

echo "== smoke: batch subcommand =="
rm -rf /tmp/sm_bt && mkdir -p /tmp/sm_bt/PWON/2023
ln -s "$DATA/PWON/2023/Audit" /tmp/sm_bt/PWON/2023/Audit 2>/dev/null
if [ -e /tmp/sm_bt/PWON/2023/Audit/instance.zip ]; then
  "$BIN" batch -d /tmp/sm_bt -o /tmp/sm_batch.csv >/dev/null 2>&1; check "batch -d" $? 0
  r=$(wc -l </tmp/sm_batch.csv); [ "$r" -gt 1 ] && ok "batch rows=$r" || bad "batch produced no rows"
  head -1 /tmp/sm_batch.csv | grep -q "^ticker,year,accounting_model" && ok "batch header" || bad "batch header"
  grep -q "^PWON,2023," /tmp/sm_batch.csv && ok "batch row has PWON" || bad "batch row missing"
  # help path
  "$BIN" batch -h >/dev/null 2>&1; check "batch -h" $? 0
  # help path
  "$BIN" fetch -h >/dev/null 2>&1; check "fetch -h" $? 0
  # An unusable downloader must be an error, not a silent no-op.
  out=$("$BIN" fetch --years 2024-2024 --bin /nonexistent 2>&1); rc=$?
  [ "$rc" -ne 0 ] && ok "fetch rejects a missing --bin (exit=$rc)" || bad "fetch accepted a missing --bin"
  "$BIN" -h >/dev/null 2>&1; check "extract -h" $? 0
else
  echo "  skip  batch (no local PWON data)"
fi

echo
echo "== smoke result: $pass passed, $fail failed =="
[ "$fail" -eq 0 ] || exit 1
