#!/usr/bin/env bash
set -euo pipefail

# Source images directory (where your files currently are)
SRC_DIR="$HOME/Documents/Pictures"

# Destination root (the emulated Takeout)
DEST_ROOT="$HOME/Documents/Takeout-Test/Takeout/Google Photos"

# Albums/folders to create
ALBUMS=("Album A" "Album B" "Photos from 2024")

# Files to include (must exist in SRC_DIR)
FILES=("banner-yt.jpg" "banner.jpg" "basic-dp.png")

echo "Creating destination at: $DEST_ROOT"
mkdir -p "$DEST_ROOT"

# Create album folders
for A in "${ALBUMS[@]}"; do
  mkdir -p "$DEST_ROOT/$A"
done

# Helper to write JSON sidecar
write_sidecar() {
  local dst_file="$1"
  local title="$2"
  local desc="$3"
  local ts="$4"   # unix seconds

  cat > "${dst_file}.json" <<EOF
{
  "title": "$(printf '%s' "$title" | sed 's/"/\\"/g')",
  "description": "$(printf '%s' "$desc" | sed 's/"/\\"/g')",
  "photoTakenTime": { "timestamp": "$ts", "formatted": "$(date -u -r "$ts" '+%Y-%m-%d %H:%M:%S UTC')" },
  "creationTime": { "timestamp": "$ts", "formatted": "$(date -u -r "$ts" '+%Y-%m-%d %H:%M:%S UTC')" },
  "geoData": { "latitude": 0.0, "longitude": 0.0, "altitude": 0.0, "latitudeSpan": 0.0, "longitudeSpan": 0.0 },
  "people": []
}
EOF
}

# Current time for consistent sidecars
NOW_TS="$(date +%s)"

echo "Copying files and creating sidecars..."

# Map each file into a couple of folders to emulate duplicates across albums
# - banner-yt.jpg -> Album A
# - banner.jpg    -> Album A and Album B
# - basic-dp.png  -> Photos from 2024
for F in "${FILES[@]}"; do
  SRC="$SRC_DIR/$F"
  if [[ ! -f "$SRC" ]]; then
    echo "Missing source file: $SRC" >&2
    exit 1
  fi
done

# 1) banner-yt.jpg -> Album A
cp "$SRC_DIR/banner-yt.jpg" "$DEST_ROOT/Album A/banner-yt.jpg"
write_sidecar "$DEST_ROOT/Album A/banner-yt.jpg" "banner-yt.jpg" "Sample sidecar for banner-yt" "$NOW_TS"

# 2) banner.jpg -> Album A and Album B (duplicate like Takeout can do)
cp "$SRC_DIR/banner.jpg" "$DEST_ROOT/Album A/banner.jpg"
write_sidecar "$DEST_ROOT/Album A/banner.jpg" "banner.jpg" "Sample sidecar for banner in Album A" "$((NOW_TS-3600))"

cp "$SRC_DIR/banner.jpg" "$DEST_ROOT/Album B/banner.jpg"
write_sidecar "$DEST_ROOT/Album B/banner.jpg" "banner.jpg" "Sample sidecar for banner in Album B" "$((NOW_TS-7200))"

# 3) basic-dp.png -> Photos from 2024 (simulate date buckets)
mkdir -p "$DEST_ROOT/Photos from 2024/2024-08-04"
cp "$SRC_DIR/basic-dp.png" "$DEST_ROOT/Photos from 2024/2024-08-04/basic-dp.png"
write_sidecar "$DEST_ROOT/Photos from 2024/2024-08-04/basic-dp.png" "basic-dp.png" "Sample sidecar for basic-dp" "$((NOW_TS-86400))"

echo "Done."
echo "Emulated Takeout path:"
echo "$DEST_ROOT"
