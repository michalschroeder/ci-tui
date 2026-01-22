.PHONY: test fmt fmt-check clippy coverage coverage-lcov test-ci ci build build-no-cache run run-local push pull clean help

IMAGE_NAME := ci-tui
VERSION ?= latest
LOCAL_IMAGE := $(IMAGE_NAME):local
REGISTRY ?= ghcr.io/lendable
FULL_IMAGE := $(REGISTRY)/$(IMAGE_NAME):$(VERSION)

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-15s\033[0m %s\n", $$1, $$2}'

# === Development ===

test: ## Run tests with nextest
	cargo nextest run

test-ci: ## Run tests with nextest (CI profile)
	cargo nextest run --profile ci

fmt: ## Format code
	cargo fmt

fmt-check: ## Check formatting without fixing
	cargo fmt -- --check

clippy: ## Run clippy lints
	cargo clippy -- -D warnings

coverage: ## Generate HTML coverage report
	cargo llvm-cov nextest --html --open

coverage-lcov: ## Generate LCOV coverage report
	cargo llvm-cov nextest --lcov --output-path lcov.info

# === CI ===

ci: fmt-check clippy test ## Run all CI checks locally

# === Docker Image ===

build: ## Build the Docker image locally
	docker build -t $(LOCAL_IMAGE) .

build-no-cache: ## Build Docker image without cache
	docker build --no-cache -t $(LOCAL_IMAGE) .

run: ## Run the TUI (pulls from registry or builds locally)
	./run.sh

run-local: build ## Build locally and run
	docker run -it --rm \
		-v $(PWD):/app \
		-v /var/run/docker.sock:/var/run/docker.sock \
		-w /app \
		$(LOCAL_IMAGE)

push: build ## Build and push to registry
	docker tag $(LOCAL_IMAGE) $(FULL_IMAGE)
	docker push $(FULL_IMAGE)

pull: ## Pull latest from registry
	docker pull $(FULL_IMAGE)

clean: ## Remove Docker images
	docker rmi $(LOCAL_IMAGE) 2>/dev/null || true
	docker rmi $(FULL_IMAGE) 2>/dev/null || true
