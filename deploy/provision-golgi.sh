#!/bin/bash
# provision-golgi.sh — One-shot provisioning for mobile golgi NUCs
#
# Usage: sudo ./provision-golgi.sh <gate-name>
# Example: sudo ./provision-golgi.sh golgiAlpha
#
# What this does:
# 1. Creates /opt/membrane and /opt/ecoPrimals directory structure
# 2. Fetches primals from WAN depot (HTTPS, no SSH required)
# 3. Installs systemd units (songbird-federation + membrane-nucleus@)
# 4. Writes gate identity for NM reconnect hook
# 5. Installs NetworkManager dispatcher for auto-mesh
# 6. Starts the mesh and NUCLEUS
#
# Prerequisites:
# - Internet access to membrane.primals.eco (HTTPS)
# - socat installed (for mesh.init)
# - Network access to VPS:7700 (TCP)

set -euo pipefail

GATE_NAME="${1:-}"
if [ -z "$GATE_NAME" ]; then
    echo "Usage: $0 <gate-name>"
    echo "Example: $0 golgiAlpha"
    exit 1
fi

VPS_PEER="${MEMBRANE_VPS_MESH_PEER:-157.230.3.183:7700}"
DEPOT_URL="https://membrane.primals.eco/depot"
ARCH="x86_64-unknown-linux-musl"
INSTALL_DIR="/opt/membrane"
SOCKET_DIR="/run/membrane"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "=== Provisioning mobile golgi: $GATE_NAME ==="
echo "    Arch: $ARCH"
echo "    VPS peer: $VPS_PEER"
echo "    Depot: $DEPOT_URL"
echo ""

# --- Phase 1: Directory structure ---
echo "[1/6] Creating directory structure..."
mkdir -p "$INSTALL_DIR" "$SOCKET_DIR" /etc/membrane /var/lib/membrane/songbird

# --- Phase 2: Fetch primals from WAN depot ---
echo "[2/6] Fetching primals from WAN depot..."
PRIMALS="beardog songbird biomeos nestgate coralreef sweetgrass squirrel loamspine rhizocrypt skunkbat petaltongue barracuda toadstool"
FETCHED=0
FAILED=0
for p in $PRIMALS; do
    echo -n "  $p... "
    if curl -sf -o "$INSTALL_DIR/$p" "$DEPOT_URL/$ARCH/$p"; then
        chmod +x "$INSTALL_DIR/$p"
        echo "OK"
        FETCHED=$((FETCHED+1))
    else
        echo "FAILED"
        FAILED=$((FAILED+1))
    fi
done
echo "  Fetched: $FETCHED, Failed: $FAILED"

if [ "$FAILED" -gt 0 ]; then
    echo "WARNING: Some primals failed to fetch. Continuing with partial deployment."
fi

# --- Phase 3: Install systemd units ---
echo "[3/6] Installing systemd units..."
if [ -d "$SCRIPT_DIR/systemd" ]; then
    cp "$SCRIPT_DIR/systemd/songbird-federation.service" /etc/systemd/system/
    cp "$SCRIPT_DIR/systemd/membrane-nucleus@.service" /etc/systemd/system/
    cp "$SCRIPT_DIR/systemd/membrane-nucleus.target" /etc/systemd/system/
    systemctl daemon-reload
    echo "  Units installed."
else
    echo "  WARNING: systemd/ not found relative to script. Install manually."
fi

# --- Phase 4: Gate identity ---
echo "[4/6] Writing gate identity..."
echo "$GATE_NAME" > /etc/membrane/gate-name
echo "  /etc/membrane/gate-name = $GATE_NAME"

# --- Phase 5: NM dispatcher hook ---
echo "[5/6] Installing NetworkManager reconnect hook..."
if [ -d /etc/NetworkManager/dispatcher.d ]; then
    if [ -f "$SCRIPT_DIR/nm-dispatcher/99-mesh-reconnect" ]; then
        cp "$SCRIPT_DIR/nm-dispatcher/99-mesh-reconnect" /etc/NetworkManager/dispatcher.d/
        chmod 755 /etc/NetworkManager/dispatcher.d/99-mesh-reconnect
        echo "  Hook installed."
    else
        echo "  WARNING: 99-mesh-reconnect not found. Install manually."
    fi
else
    echo "  NetworkManager not found — skipping (systemd-networkd or manual mesh)."
fi

# --- Phase 6: Start services ---
echo "[6/6] Starting songbird + NUCLEUS..."
systemctl enable --now songbird-federation.service 2>/dev/null || true

sleep 2

if [ -S "$SOCKET_DIR/songbird.sock" ]; then
    echo "  Songbird running. Initiating mesh..."
    MESH_CMD="{\"jsonrpc\":\"2.0\",\"method\":\"mesh.init\",\"params\":{\"node_id\":\"$GATE_NAME\",\"peers\":[\"$VPS_PEER\"]},\"id\":1}"
    RESP=$(echo "$MESH_CMD" | timeout 5 socat - "UNIX-CONNECT:$SOCKET_DIR/songbird.sock" 2>/dev/null || echo "timeout")
    echo "  mesh.init: $RESP"
else
    echo "  WARNING: Songbird socket not found. Check: journalctl -u songbird-federation"
fi

for p in $PRIMALS; do
    [ "$p" = "songbird" ] && continue
    systemctl enable --now "membrane-nucleus@${p}.service" 2>/dev/null || true
done

echo ""
echo "=== Provisioning complete ==="
echo "Gate: $GATE_NAME ($ARCH, mobile)"
echo "Mesh peer: $VPS_PEER"
echo ""
echo "Verify:"
echo "  systemctl status songbird-federation"
echo "  systemctl status 'membrane-nucleus@*'"
echo "  journalctl -u songbird-federation -f"
