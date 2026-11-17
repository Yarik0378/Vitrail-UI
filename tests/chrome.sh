#!/bin/sh
# Reference renders: headless Chrome, transparent background, after 2 s of virtual time (scripts and timers run). Usage: chrome.sh <page.html> <out.png> <w> <h> [scale]
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
"$CHROME" --headless=new --disable-gpu --hide-scrollbars --default-background-color=00000000 \
  --virtual-time-budget=2000 --force-device-scale-factor=${5:-1} --window-size=$3,$4 --screenshot="$2" "file://$(cd "$(dirname "$1")" && pwd)/$(basename "$1")" >/dev/null 2>&1
