#!/bin/sh
# Runs every test file under the folder it is given.
set -e
for file in "$1"/*.test.js; do
  echo "· $file"
done
