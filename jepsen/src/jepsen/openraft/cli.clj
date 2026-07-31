(ns jepsen.openraft.cli
  (:gen-class)
  (:require [jepsen [checker :as checker]
             [cli :as cli]
             [generator :as gen]
             [tests :as tests]]
            [jepsen.openraft [db :as openraft-db]
             [workload :as workload]]
            [jepsen.openraft.nemesis [membership :as membership]
             [partition :as partition]
             [process :as process]]))

(def nemesis-types
  #{:membership :partition :process})

(def cli-opts
  [[nil "--api-port PORT" "OpenRaft application HTTP port."
    :default 21001
    :parse-fn parse-long]

   [nil "--raft-port PORT" "OpenRaft internal Raft RPC port."
    :default 22001
    :parse-fn parse-long]

   [nil "--nemesis TYPE" "Fault type: membership, partition, or process."
    :default :partition
    :parse-fn keyword
    :validate [nemesis-types (cli/one-of nemesis-types)]]])

(defn openraft-test [opts]
  (let [database (openraft-db/db opts)
        workload (workload/workload opts)
        nemesis-type (:nemesis opts :partition)
        nemesis-package (case nemesis-type
                          :membership (membership/membership-package
                                       database
                                       opts)
                          :partition (partition/partition-package)
                          :process (process/process-package database))]
    (merge tests/noop-test
           opts
           {:name (str "openraft linearizable register "
                       (name nemesis-type))
            :db database
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
