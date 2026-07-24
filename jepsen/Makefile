COMPOSE := docker compose -f docker/docker-compose.yml
NODES ?= n1,n2,n3
CONCURRENCY ?= 3
SSH_KEY ?= /root/.ssh/openraft-jepsen
ARGS ?= --nodes $(NODES) --concurrency $(CONCURRENCY) --ssh-private-key $(SSH_KEY)

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

.PHONY: jepsen key build up test down
