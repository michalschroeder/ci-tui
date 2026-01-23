.PHONY: test fmt fmt-check clippy coverage ci build build-dev build-no-cache push pull clean help

IMAGE_NAME := ci-tui
VERSION ?= latest
LOCAL_IMAGE := $(IMAGE_NAME):local
DEV_IMAGE := $(IMAGE_NAME)-dev:latest
REGISTRY ?= docker.io
FULL_IMAGE := $(REGISTRY)/$(IMAGE_NAME):$(VERSION)

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-15s\033[0m %s\n", $$1, $$2}'

# === Development (runs in Docker using dev image) ===

test: ## Run tests with nextest
	docker run --rm -v $(PWD):/build -w /build $(DEV_IMAGE) cargo nextest run --status-level all

fmt: ## Format code
	docker run --rm -v $(PWD):/build -w /build $(DEV_IMAGE) cargo fmt

fmt-check: ## Check formatting without fixing
	docker run --rm -v $(PWD):/build -w /build $(DEV_IMAGE) cargo fmt -- --check

clippy: ## Run clippy lints
	docker run --rm -v $(PWD):/build -w /build $(DEV_IMAGE) cargo clippy -- -D warnings

coverage: ## Generate LCOV coverage report
	docker run --rm -v $(PWD):/build -w /build $(DEV_IMAGE) sh -c "rustup component add llvm-tools-preview && cargo install cargo-llvm-cov --locked && cargo llvm-cov nextest --lcov --output-path lcov.info"

update-lock: ## Update Cargo.lock with latest compatible versions
	docker run --rm -v $(PWD):/build -w /build $(DEV_IMAGE) cargo update

# === CI (mirrors GitHub Actions) ===

ci: fmt-check clippy test ## Run all CI checks locally (in Docker)

# === Docker Image ===

# Capture version info for Docker builds
GIT_HASH := $(shell git rev-parse --short HEAD 2>/dev/null || echo "unknown")
BUILD_DATE := $(shell date "+%Y-%m-%d %H:%M")
DOCKER_BUILD := docker build --build-arg CI_TUI_GIT_HASH=$(GIT_HASH) --build-arg "CI_TUI_BUILD_DATE=$(BUILD_DATE)" -t $(LOCAL_IMAGE)

build-dev: ## Build dev image (with rustfmt, clippy, nextest)
	docker build --target dev -t $(DEV_IMAGE) .

build: ## Build the Docker image locally
	$(DOCKER_BUILD) .

build-no-cache: ## Build Docker image without cache
	$(DOCKER_BUILD) --no-cache .

push: build ## Build and push to registry
	docker tag $(LOCAL_IMAGE) $(FULL_IMAGE)
	docker push $(FULL_IMAGE)

pull: ## Pull latest from registry
	docker pull $(FULL_IMAGE)

clean: ## Remove Docker images
	docker rmi $(LOCAL_IMAGE) 2>/dev/null || true
	docker rmi $(DEV_IMAGE) 2>/dev/null || true
	docker rmi $(FULL_IMAGE) 2>/dev/null || true
