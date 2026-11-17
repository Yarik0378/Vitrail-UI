#!/bin/sh
# Every page in suite.txt painted by the GPU painter (edge AA) in the lab Unity project, compared with the CPU painter
# and, when a Chrome reference exists, with Chrome (the GPU capture is premultiplied; it is unpremultiplied first).
# Usage: tests/gpu-suite.sh [python-with-PIL-and-numpy]. Unity must not have the lab project open.
set -e
T=$(cd "$(dirname "$0")" && pwd)
PY=${1:-python3}
U=/Applications/Unity/Hub/Editor/6000.3.27f1/Unity.app/Contents/MacOS/Unity
LIST=$(mktemp)
sed "s|^pages/|$T/pages/|" "$T/suite.txt" > "$LIST"
mkdir -p "$T/gpu2"
"$U" -batchmode -quit -projectPath "$T/../unity" -executeMethod VitrailLab.Editor.Lab.GpuSuite -list "$LIST" -aa 1 -out "$T/gpu2" -logFile "$T/gpu2/unity.log"
while read -r page w h s; do
  n=$(basename "$page" .html)
  "$PY" -c "
import sys; from PIL import Image; import numpy as np
g=np.asarray(Image.open(sys.argv[1]).convert('RGBA')).astype(float); a=g[...,3:]/255
rgb=np.where(a>0, g[...,:3]/np.maximum(a,1e-6), 0)
Image.fromarray(np.dstack([np.clip(rgb,0,255),g[...,3:]]).astype(np.uint8)).save(sys.argv[2])" "$T/gpu2/$n-gpuaa.png" "$T/gpu2/$n-straight.png"
  printf "%-16s cpu: %s" "$n" "$("$PY" "$T/diff.py" "$T/gpu2/$n-cpu.png" "$T/gpu2/$n-gpuaa.png" "$T/gpu2/$n-sheet.png")"
  ref="$T/ref/$n.png"; [ "$s" = 2 ] && ref="$T/ref/$n-x2.png"
  [ -f "$ref" ] && printf "   chrome: %s" "$("$PY" "$T/diff.py" "$ref" "$T/gpu2/$n-straight.png" "$T/gpu2/$n-chrome.png")"
  echo
done < "$T/suite.txt"
rm -f "$LIST"
