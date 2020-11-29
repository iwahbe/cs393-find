#!/usr/bin/env bash
set -euo pipefail

if [ ${#1} -ge 7 ]; then
    exit 1
else
    exit 0
fi
