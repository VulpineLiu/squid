#!/bin/bash
# Publish installers to the orphan git branch that is Squid's update channel.
#
#   scripts/publish-release.sh [--notes FILE] [--key FILE] [--dry-run]
#                              [--no-push] [--branch NAME]
#
# The branch contains `squid-update.json`, the exact installers it names, and
# optionally `squid-update.sig` over the manifest's bytes. Rebuilding the branch
# from scratch on every run prevents installers from accumulating in history.
set -euo pipefail
cd "$(dirname "$0")/.."

BRANCH="${SQUID_RELEASE_BRANCH:-releases}"
MANIFEST="squid-update.json"
SIGNATURE="squid-update.sig"
NOTES_FILE=""
KEY_FILE="${SQUID_RELEASE_KEY:-}"
DRY_RUN=0
NO_PUSH=0

while [ $# -gt 0 ]; do
  case "$1" in
    --notes) NOTES_FILE="$2"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    --no-push) NO_PUSH=1; shift ;;
    --branch) BRANCH="$2"; shift 2 ;;
    --key) KEY_FILE="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

VERSION=$(python3 -c "import json;print(json.load(open('src-tauri/tauri.conf.json'))['version'])")
PLAN=$(mktemp /tmp/squid-release-plan.XXXXXX)
INDEX_FILE=$(mktemp /tmp/squid-release-index.XXXXXX)
MANIFEST_PATH=$(mktemp /tmp/squid-manifest.XXXXXX)
SIGNATURE_PATH=$(mktemp /tmp/squid-signature.XXXXXX)
trap 'rm -f "$PLAN" "$INDEX_FILE" "$MANIFEST_PATH" "$SIGNATURE_PATH"' EXIT

python3 - "$VERSION" "$MANIFEST" "$NOTES_FILE" > "$PLAN" <<'PY'
import datetime, hashlib, json, pathlib, re, sys

version, manifest_name, notes_file = sys.argv[1:4]
artifacts = pathlib.Path("artifacts")
if not artifacts.is_dir():
    sys.exit("artifacts/ is missing; build a release first.")
notes = pathlib.Path(notes_file).read_text(encoding="utf-8").strip() if notes_file else ""

patterns = [
    ("macos", rf"^Squid-{re.escape(version)}-macOS-universal\.dmg$"),
    ("windows", rf"^Squid-Setup-{re.escape(version)}-win-x64\.exe$"),
]
candidates = sorted(p.name for p in artifacts.iterdir() if p.is_file())

def digest(name):
    path = artifacts / name
    return hashlib.sha256(path.read_bytes()).hexdigest(), path.stat().st_size

manifest = {
    "schema": 1,
    "version": version,
    "date": datetime.date.today().isoformat(),
    "notes": notes,
    "artifacts": {},
}
chosen = []
for platform, pattern in patterns:
    match = next((name for name in candidates if re.search(pattern, name)), None)
    if not match:
        sys.exit(
            f"missing installer for {platform} at version {version}; expected "
            f"Squid-{version}-macOS-universal.dmg and "
            f"Squid-Setup-{version}-win-x64.exe"
        )
    digest_value, size = digest(match)
    manifest["artifacts"][platform] = {
        "file": match,
        "sha256": digest_value,
        "size": size,
    }
    chosen.append(match)

unexpected = [
    name
    for name in candidates
    if re.search(r"\.(dmg|exe|msi|pkg|zip|AppImage|deb)$", name, re.IGNORECASE)
    and name not in chosen
]
if unexpected:
    sys.exit("only versioned release installers may be published; remove: " + ", ".join(unexpected))

sys.stdout.write(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")
sys.stdout.write("\0")
for name in candidates:
    if name in {manifest_name, "squid-update.sig"} or name.startswith("."):
        continue
    digest_value, size = digest(name)
    sys.stdout.write(f"{name}\t{digest_value}\t{size}\0")
PY

MANIFEST_JSON=$(python3 -c "import sys;print(sys.stdin.read().split('\0')[0],end='')" < "$PLAN")
echo "== manifest for $VERSION =="
printf '%s\n' "$MANIFEST_JSON"
printf '%s\n' "$MANIFEST_JSON" > "$MANIFEST_PATH"

if [ -n "$KEY_FILE" ]; then
  python3 scripts/release-key.py sign "$KEY_FILE" "$MANIFEST_PATH" > "$SIGNATURE_PATH"
  PUBLIC_FILE="${KEY_FILE%.ed25519}.pub"
  if [ "$PUBLIC_FILE" != "$KEY_FILE" ] && [ -f "$PUBLIC_FILE" ]; then
    python3 scripts/release-key.py verify "$PUBLIC_FILE" "$MANIFEST_PATH" "$SIGNATURE_PATH"
  fi
  echo "== manifest signed with $KEY_FILE =="
fi

echo "== building the $BRANCH tree =="
export GIT_INDEX_FILE="$INDEX_FILE"
git read-tree --empty
git update-index --add --cacheinfo "100644,$(git hash-object -w "$MANIFEST_PATH"),$MANIFEST"
if [ -s "$SIGNATURE_PATH" ]; then
  git update-index --add --cacheinfo "100644,$(git hash-object -w "$SIGNATURE_PATH"),$SIGNATURE"
fi
while IFS=$'\t' read -r -d '' name digest_value size; do
  [ -n "$name" ] || continue
  git update-index --add --cacheinfo "100644,$(git hash-object -w "artifacts/$name"),$name"
  printf '  %-42s %10s B  %s…\n' "$name" "$size" "${digest_value:0:16}"
done < <(python3 -c "
import sys
for entry in sys.stdin.read().split('\0')[1:]:
    if entry.strip():
        sys.stdout.write(entry + '\0')
" < "$PLAN")
TREE=$(git write-tree)
COMMIT=$(git commit-tree "$TREE" -m "release $VERSION")
unset GIT_INDEX_FILE

if [ "$DRY_RUN" = 1 ]; then
  echo "== dry run: $COMMIT would become $BRANCH =="
  git ls-tree -rl "$COMMIT" | while read -r mode type hash size name; do
    printf '  %12s B  %s\n' "$size" "$name"
  done
  exit 0
fi

git update-ref "refs/heads/$BRANCH" "$COMMIT"
echo "committed $VERSION as $(git rev-parse --short "$BRANCH")"

if [ "$NO_PUSH" = 1 ]; then
  echo "--no-push: committed locally on $BRANCH"
  exit 0
fi

echo "== pushing $BRANCH =="
git push --force origin "$BRANCH"
echo "published $VERSION"
