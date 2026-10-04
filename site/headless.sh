#!/bin/sh
# Serve a directory WITHOUT the cross-origin isolation headers, open the
# REPL page in it with a `#code=` example in a headless Chromium-based
# browser, and wait for its console to report `link ok`: proof that
# coi.js isolated the page and the fragment's chunks all ran.
#
# Usage: sh site/headless.sh [DIR] [PAGE] [browser]
# Defaults: tmp/site, repl/index.html, vivaldi. The page is opened through
# the debugging port; the `#` is sent as %23 because curl drops a literal
# `#` and the devtools endpoint decodes it.
set -u
dir=${1:-tmp/site} page=${2:-repl/index.html} browser=${3:-vivaldi}
port=8766 debug=9334
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$root/tmp/headless-site
rm -rf "$tmp" && mkdir -p "$tmp"
code=$(printf ': sq ( i32 -- i32 ) dup i32.mul ;\ntest sq : 3 sq -> 9\n' | base64 -w0 | tr '+/' '-_' | tr -d '=')
python3 -m http.server $port --bind 127.0.0.1 --directory "$dir" >"$tmp/serve.log" 2>&1 &
server=$!
"$browser" --headless=new --disable-gpu --no-sandbox --no-first-run \
  --enable-logging=stderr --v=0 --remote-debugging-port=$debug \
  --user-data-dir="$tmp/profile" about:blank >"$tmp/browser.log" 2>&1 &
browser_pid=$!
i=0
until curl -s "http://127.0.0.1:$debug/json/version" >/dev/null 2>&1; do
  i=$((i + 1)); [ $i -gt 50 ] && break; sleep 0.2
done
curl -s -X PUT "http://127.0.0.1:$debug/json/new?http://127.0.0.1:$port/$page%23code=$code" >/dev/null
result=""
i=0
while [ $i -lt 120 ]; do
  result=$(grep -o '"link .*", source' "$tmp/browser.log" | head -1 | sed 's/^"//; s/", source$//')
  [ -n "$result" ] && break
  i=$((i + 1)); sleep 0.5
done
kill $browser_pid $server 2>/dev/null
wait 2>/dev/null
sleep 1
rm -rf "$tmp"
echo "${result:-link TIMEOUT}"
[ "$result" = "link ok" ]
