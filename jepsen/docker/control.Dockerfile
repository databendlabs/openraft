FROM clojure:temurin-21-lein

# Retry the install so a temporary DNS or mirror failure does not cost a
# 30-minute Jepsen job. `apt-get update` only warns when it cannot fetch an
# index, and the error surfaces in `apt-get install`, so each attempt runs
# both.
RUN attempt=1; \
    until apt-get update \
       && apt-get install -y --no-install-recommends \
            git \
            openssh-client; do \
      if [ "$attempt" -ge 3 ]; then exit 1; fi; \
      echo "[control] apt-get attempt $attempt failed; retrying in 60s"; \
      attempt=$((attempt + 1)); \
      sleep 60; \
    done \
 && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /root/.ssh \
 && chmod 700 /root/.ssh

WORKDIR /openraft/jepsen

COPY jepsen/project.clj ./project.clj

# Retry dependency resolution so a temporary repository failure does not cost
# a 30-minute Jepsen job.
RUN attempt=1; \
    until lein deps; do \
      if [ "$attempt" -ge 3 ]; then exit 1; fi; \
      echo "[control] lein deps attempt $attempt failed; retrying in 60s"; \
      attempt=$((attempt + 1)); \
      sleep 60; \
    done

CMD ["sleep", "infinity"]
