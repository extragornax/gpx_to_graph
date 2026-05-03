#!/usr/bin/env bash
set -euo pipefail

SLOT_FILE=".active-slot"
CADDY_FILE="Caddyfile"
BLUE_PORT=3000
GREEN_PORT=3001
HEALTH_TIMEOUT=60
HEALTH_INTERVAL=3

current=$(cat "$SLOT_FILE" 2>/dev/null || echo "none")

case "$current" in
    blue)   next="green"; next_port=$GREEN_PORT; old_port=$BLUE_PORT  ;;
    green)  next="blue";  next_port=$BLUE_PORT;  old_port=$GREEN_PORT ;;
    *)      next="blue";  next_port=$BLUE_PORT;  old_port=$BLUE_PORT  ;;
esac

echo "=== Deploy: ${current} → ${next} (port ${next_port}) ==="

# Build image once (tagged gpx-app:latest, shared by both services)
echo "== Building image =="
docker compose build gpx-blue

# Start the new slot
echo "== Starting gpx-${next} =="
docker compose up -d --no-build "gpx-${next}"

# Health check from host
echo "== Health-checking on port ${next_port} =="
elapsed=0
while [ "$elapsed" -lt "$HEALTH_TIMEOUT" ]; do
    if curl -sf --max-time 2 "http://localhost:${next_port}/" >/dev/null 2>&1; then
        echo "== gpx-${next} healthy =="
        break
    fi
    sleep "$HEALTH_INTERVAL"
    elapsed=$((elapsed + HEALTH_INTERVAL))
done

if [ "$elapsed" -ge "$HEALTH_TIMEOUT" ]; then
    echo "ERROR: gpx-${next} not healthy after ${HEALTH_TIMEOUT}s" >&2
    docker compose logs --tail=40 "gpx-${next}" || true
    docker compose stop "gpx-${next}"
    exit 1
fi

# Update Caddy upstream to new port
if [ "$current" != "none" ]; then
    echo "== Switching Caddy to port ${next_port} =="
    sed -i '/gpx\.extragornax\.fr/,/}/ s|:'"${old_port}"'|:'"${next_port}"'|' "$CADDY_FILE"
    docker compose -f docker-compose.proxy.yml exec caddy caddy reload --config /etc/caddy/Caddyfile
fi

# Stop the old slot
if [ "$current" != "none" ]; then
    echo "== Stopping gpx-${current} =="
    docker compose stop "gpx-${current}"
fi

echo "${next}" > "$SLOT_FILE"
echo "=== Deploy complete: gpx-${next} on port ${next_port} ==="
