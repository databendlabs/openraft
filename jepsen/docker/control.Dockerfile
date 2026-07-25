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
RUN lein deps

CMD ["sleep", "infinity"]
