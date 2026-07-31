COMPOSE := docker compose -f docker/docker-compose.yml
APP_MANIFEST := openraft-test-app/Cargo.toml
NODES ?= n1,n2,n3,n4,n5
CONCURRENCY ?= 3
NEMESIS ?= partition
SSH_KEY ?= /root/.ssh/openraft-jepsen
ARGS ?= --nodes $(NODES) --concurrency $(CONCURRENCY) --nemesis $(NEMESIS) --ssh-private-key $(SSH_KEY)

app-lint:
	cargo fmt --manifest-path $(APP_MANIFEST)
	cargo clippy --no-deps --manifest-path $(APP_MANIFEST) --all-targets -- -D warnings

jepsen: build
	@echo "[jepsen] starting containers"
	@$(COMPOSE) up -d --force-recreate
	@echo "[jepsen] running unit tests"
	@$(COMPOSE) exec control lein test
	@echo "[jepsen] running linearizability test"
	@$(COMPOSE) exec control lein run test $(ARGS)

key:
	@echo "[jepsen] preparing SSH key"
	@sh docker/init-ssh-key.sh

build: key
	@echo "[jepsen] building Docker images"
	@$(COMPOSE) build

up: key
	@echo "[jepsen] starting containers"
	@$(COMPOSE) up -d --force-recreate

test:
	@echo "[jepsen] running linearizability test"
	@$(COMPOSE) exec control lein run test $(ARGS)

down:
	@echo "[jepsen] stopping containers"
	@$(COMPOSE) down

.PHONY: app-lint jepsen key build up test down
