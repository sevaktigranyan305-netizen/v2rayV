#!/bin/bash
# Install the v2rayV privileged helper and polkit policy.
# Run this once after installation: sudo ./scripts/install-helper.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

echo "Installing v2rayV helper..."

# Install helper script
install -m 755 "$PROJECT_DIR/scripts/v2rayv-helper" /usr/local/bin/v2rayv-helper

# Install polkit policy
install -m 644 "$PROJECT_DIR/polkit/com.v2rayv.vpn.policy" /usr/share/polkit-1/actions/com.v2rayv.vpn.policy

echo "Done. Password will be cached for 5 minutes after first authentication."
