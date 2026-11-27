#!/usr/bin/env bash
# Virus-scans every runtime exe that compiled (Microsoft Defender, a custom scan of each file).
# Usage: bash scripts/scan-exes.sh [minutes]  (only exes built in the last N minutes; default: all)
# Prints one line per threat and a summary; exit 1 if Defender reports a threat.
mp="/c/Program Files/Windows Defender/MpCmdRun.exe"
[ -x "$mp" ] || { echo "scan: Defender's MpCmdRun.exe not found"; exit 2; }
src="$LOCALAPPDATA/6ixpack/src"
age=${1:+-mmin -$1}
n=0; bad=0
# 6ixpack is excluded from Defender (0x80508023: path excluded), so each exe is scanned as a copy outside it.
tmp="$TEMP/i95-scan"; mkdir -p "$tmp"
while IFS= read -r exe; do
  n=$((n+1))
  cp -f "$exe" "$tmp/scan.exe" 2>/dev/null || { echo "scan: cannot copy $exe"; continue; }
  out=$("$mp" -Scan -ScanType 3 -File "$(cygpath -w "$tmp/scan.exe")" -DisableRemediation 2>&1)
  if echo "$out" | grep -q 'found no threats'; then :; else bad=$((bad+1)); echo "THREAT? $exe"; echo "$out" | grep -iE 'threat|virus' | head -3; fi
done < <(find "$src" -path '*/target/*' -maxdepth 5 -name '*.exe' -not -path '*/deps/*' -not -path '*/build/*' $age 2>/dev/null)
rm -f "$tmp/scan.exe"
echo "scan: $n exes, $bad flagged"
[ $bad -eq 0 ]
