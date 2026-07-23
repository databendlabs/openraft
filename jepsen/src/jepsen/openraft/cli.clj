(ns jepsen.openraft.cli
  (:gen-class)
  (:require [jepsen [checker :as checker]
                    [cli :as cli]
                    [generator :as gen]
                    [tests :as tests]]
            [jepsen.openraft [db :as openraft-db]
                             [workload :as workload]]
            [jepsen.openraft.nemesis [partition :as partition]]))

(def cli-opts
  [[nil "--api-port PORT" "OpenRaft application HTTP port."
    :default 21001
    :parse-fn parse-long]

   [nil "--raft-port PORT" "OpenRaft internal Raft RPC port."
    :default 22001
    :parse-fn parse-long]])

(defn openraft-test [opts]
  (let [workload (workload/workload opts)
        nemesis-package (partition/partition-package)]
    (merge tests/noop-test
           opts
           {:name "openraft linearizable register"
            :db (openraft-db/db opts)
            :client (:client workload)
            :nemesis (:nemesis nemesis-package)
            :generator (gen/phases
                         (gen/shortest-any
                           (gen/nemesis
                             (gen/phases
                               (gen/time-limit
                                 (:time-limit opts)
                                 (:generator nemesis-package))
                               (:final-generator nemesis-package)))
                           (:generator workload))
                         (:final-generator workload))
            :checker (checker/compose
                        {:stats (checker/stats)
                         :nemesis (:checker nemesis-package)
                         :workload (:checker workload)})})))

(defn -main [& args]
  (cli/run! (cli/single-test-cmd {:test-fn openraft-test
                                  :opt-spec cli-opts
                                  :usage (cli/test-usage)})
            args))
