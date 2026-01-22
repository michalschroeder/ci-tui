#!/bin/bash
# CI TUI Runner
# Runs the CI TUI tool via Docker

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

# Configure your registry here (GitHub Container Registry example)
# Change this to your registry: docker.io/yourorg, ghcr.io/yourorg, etc.
REGISTRY="${CI_TUI_REGISTRY:-ghcr.io/lendable}"
IMAGE_NAME="ci-tui"
IMAGE_TAG="${CI_TUI_VERSION:-latest}"
FULL_IMAGE="${REGISTRY}/${IMAGE_NAME}:${IMAGE_TAG}"
LOCAL_IMAGE="${IMAGE_NAME}:local"
CONFIG_FILE="${CI_TUI_CONFIG:-./tools/ci/ci-config.yaml}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

usage() {
    echo "Usage: $0 [options]"
    echo ""
    echo "Options:"
    echo "  --build       Build locally instead of pulling from registry"
    echo "  --push        Build and push to registry (requires auth)"
    echo "  --pull        Force pull latest image from registry"
    echo "  --simple, -s  Run in simple console mode (no TUI)"
    echo "  --help        Show this help message"
    echo ""
    echo "Environment variables:"
    echo "  CI_TUI_REGISTRY   Docker registry (default: ghcr.io/lendable)"
    echo "  CI_TUI_VERSION    Image tag (default: latest)"
    echo "  CI_TUI_CONFIG     Path to config file (default: ./tools/ci/ci-config.yaml)"
    echo ""
    echo "Keyboard shortcuts in TUI:"
    echo "  q             Quit"
    echo "  ↑↓ / j/k      Navigate checks"
    echo "  f             Filter failed checks"
    echo "  a             Show all checks"
    echo "  c             Copy command to clipboard"
    echo "  e             Expand/collapse full command"
    echo "  t             Trigger on-demand test (functional/integration)"
    echo "  r             Retry selected check"
    echo "  R (Shift+r)   Retry ALL (refresh git & rerun all)"
    echo "  x             Auto-fix selected check"
    echo "  X (Shift+x)   Auto-fix ALL failed checks"
    echo "  PageUp/Down   Scroll output"
}

BUILD=false
PUSH=false
PULL=false
TUI_ARGS=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --build)
            BUILD=true
            shift
            ;;
        --push)
            BUILD=true
            PUSH=true
            shift
            ;;
        --pull)
            PULL=true
            shift
            ;;
        --simple|-s)
            TUI_ARGS="$TUI_ARGS --simple"
            shift
            ;;
        --help)
            usage
            exit 0
            ;;
        *)
            echo -e "${RED}Unknown option: $1${NC}"
            usage
            exit 1
            ;;
    esac
done

cd "$SCRIPT_DIR"

# Determine which image to use
if [[ "$BUILD" == "true" ]]; then
    echo -e "${YELLOW}Building CI TUI Docker image locally...${NC}"
    # Use BuildKit for faster builds with cache mounts
    DOCKER_BUILDKIT=1 docker build -t "$LOCAL_IMAGE" .

    if [[ "$PUSH" == "true" ]]; then
        echo -e "${YELLOW}Tagging and pushing to registry...${NC}"
        docker tag "$LOCAL_IMAGE" "$FULL_IMAGE"
        docker push "$FULL_IMAGE"
        echo -e "${GREEN}Pushed to: $FULL_IMAGE${NC}"
    fi

    RUN_IMAGE="$LOCAL_IMAGE"
else
    # Try to use registry image
    if [[ "$PULL" == "true" ]] || ! docker image inspect "$FULL_IMAGE" &>/dev/null; then
        echo -e "${YELLOW}Pulling CI TUI from registry...${NC}"
        if docker pull "$FULL_IMAGE" 2>/dev/null; then
            echo -e "${GREEN}Pulled: $FULL_IMAGE${NC}"
        else
            echo -e "${YELLOW}Could not pull from registry, building locally...${NC}"
            DOCKER_BUILDKIT=1 docker build -t "$LOCAL_IMAGE" .
            FULL_IMAGE="$LOCAL_IMAGE"
        fi
    fi
    RUN_IMAGE="$FULL_IMAGE"
fi

echo -e "${GREEN}Starting CI TUI...${NC}"

# Ensure we have the latest remote refs for comparison
git -C "$PROJECT_ROOT" fetch origin development 2>/dev/null || true

# Run the TUI with:
# - Interactive TTY for the TUI
# - Mount project root for git operations and config
# - Mount Docker socket for running Docker commands
docker run -it --rm \
    -v "$PROJECT_ROOT:/app" \
    -v /var/run/docker.sock:/var/run/docker.sock \
    -w /app \
    "$RUN_IMAGE" \
    --config "$CONFIG_FILE" $TUI_ARGS
