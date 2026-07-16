(defproject jepsen.openraft "0.1.0-SNAPSHOT"
  :description "Jepsen tests for OpenRaft"
  :url "https://github.com/databendlabs/openraft"
  :license {:name "Apache License 2.0"
            :url "https://www.apache.org/licenses/LICENSE-2.0"}
  :dependencies [[org.clojure/clojure "1.12.5"]
                 [jepsen "0.3.12-SNAPSHOT"]
                 [cheshire "5.6.3"]]
  :jvm-opts ["-Djava.awt.headless=true"
             "-server"]
  :repl-options {:init-ns jepsen.openraft.cli}
  :aot [jepsen.openraft.cli]
  :main jepsen.openraft.cli)
