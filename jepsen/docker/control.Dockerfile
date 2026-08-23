FROM clojure:temurin-21-lein

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      git \
      openssh-client \
 && rm -rf /var/lib/apt/lists/*

RUN mkdir -p /root/.ssh \
 && chmod 700 /root/.ssh

WORKDIR /openraft/jepsen

COPY jepsen/project.clj ./project.clj

# repo1.maven.org sometimes answers 403 to a whole runner IP, so a single
# pass fails every Central artifact at once. The 2026-08-23 `pause` job died
# that way while its four sibling matrix jobs fetched the same artifacts
# cleanly. Retry the resolution so one Central hiccup does not cost a
# 30-minute Jepsen job.
RUN attempt=1; \
    until lein deps; do \
      if [ "$attempt" -ge 3 ]; then exit 1; fi; \
      echo "[control] lein deps attempt $attempt failed; retrying in 60s"; \
      attempt=$((attempt + 1)); \
      sleep 60; \
    done

CMD ["sleep", "infinity"]
