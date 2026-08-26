#!/usr/bin/env bash
# LuxMini — drive the Mac front LED from the shell.
#
# Requires the LuxMini app running with the local API enabled
# (Settings > General). Two equivalent ways: the CLI, or plain curl.
set -euo pipefail

# --- Via the `luxmini` CLI ---------------------------------------------------
luxmini get
luxmini brightness 200
luxmini effect pulse
luxmini off

# --- Or directly against the local API (no CLI needed) -----------------------
PORT="${LUXMINI_API_PORT:-4470}"
AUTH=()
[ -n "${LUXMINI_API_TOKEN:-}" ] && AUTH=(-H "Authorization: Bearer ${LUXMINI_API_TOKEN}")

curl -s "${AUTH[@]}" "http://127.0.0.1:${PORT}/led"
curl -s "${AUTH[@]}" -X POST "http://127.0.0.1:${PORT}/led" -d '{"brightness":128}'
curl -s "${AUTH[@]}" -X POST "http://127.0.0.1:${PORT}/led" -d '{"on":false}'
