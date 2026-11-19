#!/bin/sh
# Packs a built binary with LICENSE and README.md as dist/NAME.tar.gz, or as
# dist/NAME.zip when it is a Windows .exe.
set -eu
bin=$1 name=$2
mkdir -p "dist/$name"
if [ -f "$bin.exe" ]; then
  cp "$bin.exe" LICENSE README.md "dist/$name/"
  (cd dist && powershell -NoProfile -Command "Compress-Archive -Force -Path '$name' -DestinationPath '$name.zip'")
else
  cp "$bin" LICENSE README.md "dist/$name/"
  tar -C dist -czf "dist/$name.tar.gz" "$name"
fi
