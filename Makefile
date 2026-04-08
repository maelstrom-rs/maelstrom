.PHONY: help build test check fmt lint clean \
       dev-up dev-down dev-logs \
       stack-up stack-down stack-logs \
       db-init db-drop db-shell \
       complement complement-list complement-build \
       run

# -- Config (override with env vars or .env) -----------------------------
DB_HOST   ?= localhost:8100
DB_USER   ?= root
DB_PASS   ?= root
DB_NS     ?= maelstrom
DB_NAME   ?= maelstrom
NATS_URL  ?= nats://localhost:4322

DB_CONN   := --conn http://$(DB_HOST) --user $(DB_USER) --pass $(DB_PASS) --ns $(DB_NS) --db $(DB_NAME)

# -- Help ---------------------------------------------------------------
help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | sort | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-20s\033[0m %s\n", $$1, $$2}'

# -- Build & Test --------------------------------------------------------
build: ## Build the project
	cargo build

run: ## Run the server (requires config/local.toml and services running)
	cargo run

test: ## Run all tests
	cargo test

check: ## Type-check without building
	cargo check

fmt: ## Format code
	cargo fmt --all

lint: ## Run clippy lints
	cargo clippy --all-targets --all-features

clean: ## Clean build artifacts
	cargo clean

# -- Dev Stack (lightweight: SurrealDB standalone + RustFS) --------------
dev-up: ## Start lightweight dev services (SurrealDB + RustFS)
	docker compose -f docker-compose.dev.yml up -d

dev-down: ## Stop dev services
	docker compose -f docker-compose.dev.yml down

dev-logs: ## Tail dev service logs
	docker compose -f docker-compose.dev.yml logs -f

dev-reset: ## Stop dev services and wipe all data
	docker compose -f docker-compose.dev.yml down -v

# -- Full Stack (TiKV cluster + SurrealDB + RustFS) ---------------------
stack-up: ## Start full clustered stack (TiKV + SurrealDB + RustFS)
	docker compose up -d

stack-down: ## Stop full stack
	docker compose down

stack-logs: ## Tail full stack logs
	docker compose logs -f

stack-reset: ## Stop full stack and wipe all data
	docker compose down -v

# -- Database ------------------------------------------------------------
db-init: ## Bootstrap the schema against a running SurrealDB
	surreal sql $(DB_CONN) < db/schema.surql

db-drop: ## Drop all data (keeps schema)
	surreal sql $(DB_CONN) --hide-welcome <<< \
		"REMOVE TABLE user; REMOVE TABLE profile; REMOVE TABLE device; REMOVE TABLE room; REMOVE TABLE membership; REMOVE TABLE event; REMOVE TABLE event_edge; REMOVE TABLE server_key;"

db-shell: ## Open interactive SurrealDB shell
	surreal sql $(DB_CONN) --pretty

# -- Complement (Matrix spec compliance tests) --------------------------
complement: ## Run Complement CS API tests (requires Go + Docker)
	-./scripts/complement.sh

complement-filter: ## Run Complement tests matching FILTER (e.g. make complement-filter FILTER=TestLogin)
	./scripts/complement.sh $(FILTER)

complement-report: ## Re-generate report from last Complement run
	./scripts/complement.sh -report

complement-list: ## List available Complement tests
	./scripts/complement.sh -list

complement-build: ## Build only the Complement Docker image (no test run)
	docker build -t complement-maelstrom -f Dockerfile.complement .

# -- Shortcuts -----------------------------------------------------------
dev: dev-up ## Alias for dev-up
up: dev-up ## Alias for dev-up
down: dev-down ## Alias for dev-down
t: test ## Alias for test
