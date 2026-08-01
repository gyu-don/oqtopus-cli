SHELL := bash
.SHELLFLAGS := -eu -o pipefail -c
.DEFAULT_GOAL := help

CARGO ?= cargo
CHARACTERIZATION_TEST ?= characterization

.PHONY: install test test-bash diff-backend-configs docs-lint docs-build docs-serve help

install: ## Install dependencies and configure git hooks and commit template
	@uv sync --all-groups
	@if [ -d .git ]; then \
		git config --local commit.template .gitmessage; \
	fi
	@chmod +x scripts/diff-backend-configs.sh

test: ## Run all Rust tests (characterization tests use OQTOPUS_TEST_BIN when set)
	@$(CARGO) test --locked

test-bash: ## Run characterization tests against the production Bash CLI
	@unset OQTOPUS_TEST_BIN; \
		$(CARGO) test --locked --test $(CHARACTERIZATION_TEST)

diff-backend-configs: ## Compare backend template configs against upstream repositories
	@scripts/diff-backend-configs.sh

docs-lint: ## Run documentation linting
	@uv run pymarkdownlnt scan -r docs

docs-build: ## Build documentation
	@uv run mkdocs build

docs-serve: ## Serve documentation locally
	@uv run mkdocs serve

help: ## Show help message
	@echo "Usage: make [target]"
	@echo ""
	@echo "Available targets:"
	@echo ""
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(filter-out .env,$(MAKEFILE_LIST)) | sort | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-30s\033[0m %s\n", $$1, $$2}'
