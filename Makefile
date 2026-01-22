.PHONY: build build-no-cache run run-local push pull dev check fmt clippy test clean help

IMAGE_NAME := ci-tui
VERSION ?= latest
LOCAL_IMAGE := $(IMAGE_NAME):local
REGISTRY ?= ghcr.io/lendable
FULL_IMAGE := $(REGISTRY)/$(IMAGE_NAME):$(VERSION)
PROJECT_ROOT := $(shell cd ../../.. && pwd)
CONFIG_FILE ?= ./tools/ci/ci-config.yaml

help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-15s\033[0m %s\n", $$1, $$2}'

build: ## Build the Docker image locally
	docker build -t $(LOCAL_IMAGE) .

run: ## Run the TUI (pulls from registry or builds locally)
	./run.sh

run-local: build ## Build locally and run
	docker run -it --rm \
		-v $(PROJECT_ROOT):/app \
		-v /var/run/docker.sock:/var/run/docker.sock \
		-w /app \
		$(LOCAL_IMAGE) --config $(CONFIG_FILE)

push: build ## Build and push to registry
	docker tag $(LOCAL_IMAGE) $(FULL_IMAGE)
	docker push $(FULL_IMAGE)

pull: ## Pull latest from registry
	docker pull $(FULL_IMAGE)

dev: ## Run with local cargo (requires Rust installed)
	cd $(PROJECT_ROOT) && cargo run --manifest-path tools/ci/tui/Cargo.toml -- --config $(CONFIG_FILE)

check: ## Run cargo check (in Docker)
	docker run --rm -v $(PWD):/build -w /build rust:alpine cargo check

fmt: ## Format code (in Docker)
	docker run --rm -v $(PWD):/build -w /build rust:latest cargo fmt

clippy: ## Run clippy lints (in Docker)
	docker run --rm -v $(PWD):/build -w /build rust:latest sh -c "rustup component add clippy && cargo clippy"

test: ## Run tests (in Docker)
	docker run --rm -v $(PWD):/build -w /build rust:alpine cargo test

build-no-cache: ## Build Docker image without cache
	docker build --no-cache -t $(LOCAL_IMAGE) .

clean: ## Remove Docker images
	docker rmi $(LOCAL_IMAGE) 2>/dev/null || true
	docker rmi $(FULL_IMAGE) 2>/dev/null || true
