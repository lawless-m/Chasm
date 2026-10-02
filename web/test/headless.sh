#!/bin/sh
# Run a page of web/ in a headless Chromium-based browser and wait for its
# console to report MARKER ok or MARKER FAIL. Used for checks that need a
# real browser (WasmGC under node needs node 22).
#
# Usage: sh web/test/headless.sh test/structs.html STRUCTS [browser]
# The browser defaults to vivaldi. A fresh profile opens a welcome page at
# start-up, so the page is opened through the debugging port instead.
set -u
page=$1 marker=$2 browser=${3:-vivaldi}
port=8765 debug=9333
root=$(cd "$(dirname "$0")/../.." && pwd)
tmp=$root/tmp/headless
rm -rf "$tmp" && mkdir -p "$tmp"
python3 "$root/web/serve.py" $port >"$tmp/serve.log" 2>&1 &
server=$!
"$browser" --headless=new --disable-gpu --no-sandbox --no-first-run \
  --enable-logging=stderr --v=0 --remote-debugging-port=$debug \
  --user-data-dir="$tmp/profile" about:blank >"$tmp/browser.log" 2>&1 &
browser_pid=$!
i=0
until curl -s "http://127.0.0.1:$debug/json/version" >/dev/null 2>&1; do
  i=$((i + 1)); [ $i -gt 50 ] && break; sleep 0.2
done
curl -s -X PUT "http://127.0.0.1:$debug/json/new?http://127.0.0.1:$port/$page" >/dev/null
result=""
i=0
while [ $i -lt 120 ]; do
  result=$(grep -o "\"$marker .*\", source" "$tmp/browser.log" | head -1 | sed 's/^"//; s/", source$//')
  [ -n "$result" ] && break
  i=$((i + 1)); sleep 0.5
done
kill $browser_pid $server 2>/dev/null
wait 2>/dev/null
sleep 1
rm -rf "$tmp/profile"
echo "${result:-$marker TIMEOUT}"
[ "$result" = "$marker ok" ]
