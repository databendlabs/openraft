COMPOSE := docker compose -f docker/docker-compose.yml
APP_MANIFEST := openraft-test-app/Cargo.toml
NODES ?= n1,n2,n3,n4,n5
CONCURRENCY ?= 3
NEMESIS ?= chaos
SCENARIO ?= all
LIVENESS_SCENARIOS := bridge-partition two-leaf-partition
comma := ,
ifeq ($(SCENARIO),all)
SELECTED_SCENARIOS := $(LIVENESS_SCENARIOS)
else
SELECTED_SCENARIOS := $(subst $(comma), ,$(SCENARIO))
endif
SSH_KEY ?= /root/.ssh/openraft-jepsen
ARGS ?= --nodes $(NODES) --concurrency $(CONCURRENCY) --ssh-private-key $(SSH_KEY)

ifneq ($(SEED),)
override ARGS += --seed $(SEED)
endif

ifneq ($(SNAPSHOT_THRESHOLD),)
override ARGS += --snapshot-threshold $(SNAPSHOT_THRESHOLD)
endif

ifneq ($(PACKET_MODE),)
PACKET_ARGS := --packet-mode $(PACKET_MODE)
endif

app-lint:
	cargo fmt --check --manifest-path $(APP_MANIFEST)
	cargo clippy --no-deps --manifest-path $(APP_MANIFEST) --all-targets -- -D warnings

app-test:
	cargo test --manifest-path $(APP_MANIFEST)

clojure-lint:
	clj-kondo --lint src test
	cljfmt check src test

lint: app-lint clojure-lint

unit-test:
	lein test

jepsen: build
	@echo "[jepsen] starting containers"
	@$(COMPOSE) up -d --force-recreate
	@echo "[jepsen] running unit tests"
	@$(COMPOSE) exec control lein test
	@$(MAKE) test

key:
	@echo "[jepsen] preparing SSH key"
	@sh docker/init-ssh-key.sh

build: key
	@echo "[jepsen] building Docker images"
	@$(COMPOSE) build

up: key
	@echo "[jepsen] starting containers"
	@$(COMPOSE) up -d --force-recreate

# Recursive calls in one recipe stay sequential even with make -j.
# Preserve failure while still attempting the other test category.
test:
	@status=0; $(MAKE) safety || status=1; \
	$(MAKE) liveness || status=1; exit $$status

safety:
	@echo "[jepsen] running safety test"
	@$(COMPOSE) exec control lein run test $(ARGS) --nemesis=$(NEMESIS) $(PACKET_ARGS)

liveness:
	@test -n "$(SELECTED_SCENARIOS)" && test -z "$(filter-out $(LIVENESS_SCENARIOS),$(SELECTED_SCENARIOS))" || \
	{ echo "Unknown scenario: $(SCENARIO). Available: all, $(LIVENESS_SCENARIOS)"; exit 1; }
	@status=0; for scenario in $(SELECTED_SCENARIOS); do \
		echo "[jepsen] running liveness scenario $$scenario"; \
		$(COMPOSE) exec control lein run test $(ARGS) --liveness=$$scenario || status=1; \
	done; exit $$status

down:
	@echo "[jepsen] stopping containers"
	@$(COMPOSE) down

.PHONY: safety liveness app-lint app-test clojure-lint lint unit-test jepsen key build up test down
