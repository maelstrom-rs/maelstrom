#!/usr/bin/env bash
# Run Complement test suite against Maelstrom and generate a report.
#
# Usage:
#   ./scripts/complement.sh              # Run all CS API tests
#   ./scripts/complement.sh TestLogin    # Run tests matching pattern
#   ./scripts/complement.sh -list        # List available tests
#   ./scripts/complement.sh -report      # Re-generate report from last run

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
COMPLEMENT_DIR="${COMPLEMENT_DIR:-$PROJECT_DIR/../complement}"
IMAGE_NAME="complement-maelstrom"
RESULTS_FILE="$PROJECT_DIR/complement-results.json"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

generate_report() {
    python3 -c "
import json, sys
from collections import defaultdict

results = {}
errors = {}
with open('$RESULTS_FILE') as f:
    for line in f:
        try:
            d = json.loads(line)
        except:
            continue
        test = d.get('Test')
        action = d.get('Action')
        if not test:
            continue
        if action in ('pass', 'fail', 'skip'):
            results[test] = action
        if action == 'output' and test in results and results.get(test) != 'pass':
            text = d.get('Output', '').strip()
            if text and ('Error' in text or 'FAIL' in text or 'got' in text or 'expected' in text or 'status' in text):
                if test not in errors:
                    errors[test] = text

passed = sorted([t for t, a in results.items() if a == 'pass'])
failed = sorted([t for t, a in results.items() if a == 'fail'])
skipped = sorted([t for t, a in results.items() if a == 'skip'])
total = len(results)

# Categorize by feature area
categories = {
    'Registration': [], 'Login/Auth': [], 'Rooms': [], 'Messages': [],
    'Sync': [], 'State': [], 'Members': [], 'Profile': [], 'Account': [],
    'Keys/E2EE': [], 'Typing': [], 'Receipts': [], 'Presence': [],
    'Media': [], 'Search': [], 'Push': [], 'Relations': [],
    'Federation': [], 'Other': []
}

def categorize(name):
    n = name.lower()
    if 'registr' in n or 'register' in n: return 'Registration'
    if 'login' in n or 'logout' in n or 'password' in n or 'deactivate' in n: return 'Login/Auth'
    if 'room' in n and ('create' in n or 'alias' in n or 'canonical' in n or 'forget' in n or 'invite' in n or 'summary' in n): return 'Rooms'
    if 'message' in n or 'send' in n or 'txn' in n or 'fetch' in n or 'redact' in n: return 'Messages'
    if 'sync' in n or 'gap' in n or 'filter' in n: return 'Sync'
    if 'state' in n or 'power' in n: return 'State'
    if 'member' in n or 'join' in n or 'leave' in n or 'kick' in n or 'ban' in n: return 'Members'
    if 'profile' in n or 'display' in n or 'avatar' in n: return 'Profile'
    if 'account' in n: return 'Account'
    if 'key' in n or 'e2e' in n or 'device' in n or 'upload_key' in n or 'to_device' in n: return 'Keys/E2EE'
    if 'typing' in n: return 'Typing'
    if 'receipt' in n or 'read_marker' in n: return 'Receipts'
    if 'presence' in n: return 'Presence'
    if 'media' in n or 'content' in n or 'upload' in n or 'url_preview' in n or 'image' in n: return 'Media'
    if 'search' in n: return 'Search'
    if 'push' in n: return 'Push'
    if 'relation' in n or 'thread' in n: return 'Relations'
    if 'federation' in n or 'over_federation' in n or 'outbound' in n or 'inbound' in n or 'backfill' in n: return 'Federation'
    return 'Other'

for t, a in results.items():
    cat = categorize(t)
    categories[cat].append((t, a))

print()
print('╔══════════════════════════════════════════════════════╗')
print('║         Complement CS API Test Report               ║')
print('╠══════════════════════════════════════════════════════╣')
print(f'║  Total:   {total:<42} ║')
print(f'║  Passed:  \033[32m{len(passed):<42}\033[0m ║')
print(f'║  Failed:  \033[31m{len(failed):<42}\033[0m ║')
print(f'║  Skipped: \033[33m{len(skipped):<42}\033[0m ║')
rate = f'{len(passed)/total*100:.1f}%' if total > 0 else '0%'
print(f'║  Pass rate: \033[36m{rate:<40}\033[0m ║')
print('╚══════════════════════════════════════════════════════╝')
print()

# Category breakdown
print('┌─────────────────────┬───────┬───────┬───────┬────────┐')
print('│ Category            │ Total │  Pass │  Fail │   Rate │')
print('├─────────────────────┼───────┼───────┼───────┼────────┤')
for cat in categories:
    tests = categories[cat]
    if not tests:
        continue
    p = sum(1 for _, a in tests if a == 'pass')
    f = sum(1 for _, a in tests if a == 'fail')
    t = len(tests)
    r = f'{p/t*100:.0f}%' if t > 0 else '-'
    color = '\033[32m' if p == t else '\033[31m' if p == 0 else '\033[33m'
    print(f'│ {cat:<19} │ {t:>5} │ \033[32m{p:>5}\033[0m │ \033[31m{f:>5}\033[0m │ {color}{r:>6}\033[0m │')
print('└─────────────────────┴───────┴───────┴───────┴────────┘')
print()

if passed:
    print(f'\033[32m✓ Passed ({len(passed)}):\033[0m')
    for t in passed:
        print(f'  {t}')
    print()

if failed:
    print(f'\033[31m✗ Failed ({len(failed)}):\033[0m')
    for t in failed:
        err = errors.get(t, '')
        if err:
            print(f'  {t}')
            print(f'    \033[2m{err[:120]}\033[0m')
        else:
            print(f'  {t}')
    print()

print(f'Full JSON results: $RESULTS_FILE')
"
}

# -- Handle -report (just re-generate from existing results) --
if [ "${1:-}" = "-report" ]; then
    if [ ! -f "$RESULTS_FILE" ]; then
        echo -e "${RED}No results file found. Run tests first: make complement${NC}"
        exit 1
    fi
    generate_report
    exit 0
fi

# -- Check prerequisites --
if ! command -v go &>/dev/null; then
    echo -e "${RED}Error: Go is required to run Complement. Install from https://go.dev/dl/${NC}"
    exit 1
fi

if ! command -v docker &>/dev/null; then
    echo -e "${RED}Error: Docker is required to run Complement.${NC}"
    exit 1
fi

# -- Clone Complement if needed --
if [ ! -d "$COMPLEMENT_DIR" ]; then
    echo -e "${YELLOW}Cloning Complement to $COMPLEMENT_DIR...${NC}"
    git clone https://github.com/matrix-org/complement.git "$COMPLEMENT_DIR"
fi

# -- Build Maelstrom Complement image --
echo -e "${YELLOW}Building Maelstrom Complement Docker image...${NC}"
docker build -t "$IMAGE_NAME" -f "$PROJECT_DIR/Dockerfile.complement" "$PROJECT_DIR"

# -- Run tests --
cd "$COMPLEMENT_DIR"

export COMPLEMENT_BASE_IMAGE="$IMAGE_NAME"
export COMPLEMENT_DEBUG="${COMPLEMENT_DEBUG:-0}"
export COMPLEMENT_ALWAYS_PRINT_SERVER_LOGS="${COMPLEMENT_ALWAYS_PRINT_SERVER_LOGS:-0}"

# macOS Docker Desktop uses a non-standard socket path
if [ -z "${DOCKER_HOST:-}" ] && [ -S "$HOME/.docker/run/docker.sock" ]; then
    export DOCKER_HOST="unix://$HOME/.docker/run/docker.sock"
fi

FILTER="${1:-}"

if [ "$FILTER" = "-list" ]; then
    echo -e "${YELLOW}Available Complement tests:${NC}"
    go test -list '.*' ./tests/... 2>/dev/null | grep "^Test"
    exit 0
fi

echo -e "${YELLOW}Running Complement tests (CS API + Federation)...${NC}"
if [ -n "$FILTER" ]; then
    echo -e "${YELLOW}Filter: $FILTER${NC}"
fi

RUN_ARG=""
if [ -n "$FILTER" ]; then
    RUN_ARG="-run $FILTER"
fi

# Run ALL tests — CS API and federation
# -parallel: run test cases concurrently (default is GOMAXPROCS which is fine)
# -timeout: generous timeout for the full suite
set +e
go test -json -count=1 -timeout 30m -parallel "${COMPLEMENT_PARALLEL:-4}" $RUN_ARG ./tests/... > "$RESULTS_FILE" 2>&1
TEST_EXIT=$?
set -e

# -- Generate report --
generate_report

exit $TEST_EXIT
