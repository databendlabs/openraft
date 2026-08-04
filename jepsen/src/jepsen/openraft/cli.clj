(ns jepsen.openraft.cli
  (:gen-class)
  (:require [jepsen [checker :as checker]
             [cli :as cli]
             [generator :as gen]
             [random :as random]
             [tests :as tests]]
            [jepsen.store :as store]
            [jepsen.openraft [db :as openraft-db]
             [workload :as workload]]
            [jepsen.openraft.nemesis [membership :as membership]
             [partition :as partition]
             [process :as process]]))

(def nemesis-types
  #{:membership :partition :pause :process})

(def ^:private node-crash-pattern
  #"(panicked at|fatal runtime error)")

(def cli-opts
  [[nil "--api-port PORT" "OpenRaft application HTTP port."
    :default 21001
    :parse-fn parse-long]

   [nil "--raft-port PORT" "OpenRaft internal Raft RPC port."
    :default 22001
    :parse-fn parse-long]

   [nil "--snapshot-threshold COUNT"
    "Committed logs between snapshots."
    :default openraft-db/default-snapshot-threshold
    :parse-fn parse-long
    :validate [#(and (some? %) (pos? %))
               "Must be a positive integer."]]

   [nil "--nemesis TYPE"
    "Fault type: membership, partition, pause, or process."
    :default :partition
    :parse-fn keyword
    :validate [nemesis-types (cli/one-of nemesis-types)]]

   [nil "--seed SEED" "Seed for Jepsen random choices."
    :parse-fn parse-long
    :validate [some? "Must be an integer."]]])

(defn- ensure-random-seed [parsed]
  (update parsed :options
          (fn [options]
            (let [seed (or (:seed options)
                           (random/long Long/MAX_VALUE))]
              (random/set-seed! seed)
              (assoc options :seed seed)))))

(defn- random-seed-checker []
  (reify checker/Checker
    (check [_ test _history _opts]
      {:valid? true
       :seed (:seed test)})))

(defn- strict-unhandled-exceptions []
  (let [delegate (checker/unhandled-exceptions)]
    (reify checker/Checker
      (check [_ test history opts]
        (let [result (checker/check delegate test history opts)]
          (if (seq (:exceptions result))
            (assoc result :valid? :unknown)
            result))))))

(defn- required-log-file-pattern [pattern filename]
  (let [delegate (checker/log-file-pattern pattern filename)]
    (reify checker/Checker
      (check [_ test history opts]
        (let [missing-nodes (->> (:nodes test)
                                 (remove (fn [node]
                                           (.isFile
                                            ^java.io.File
                                            (store/path test node filename))))
                                 vec)]
          (if (seq missing-nodes)
            {:valid? :unknown
             :filename filename
             :missing-nodes missing-nodes}
            (checker/check delegate test history opts)))))))

(defn openraft-test [opts]
  (let [database (openraft-db/db opts)
        workload (workload/workload opts)
        nemesis-type (:nemesis opts :partition)
        nemesis-package (case nemesis-type
                          :membership (membership/membership-package
                                       database
                                       opts)
                          :partition (partition/partition-package)
                          :pause (process/pause-package database)
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
                      {:seed (random-seed-checker)
                       :stats (checker/stats)
                       :exceptions (strict-unhandled-exceptions)
                       :crash (required-log-file-pattern
                               node-crash-pattern
                               "openraft.log")
                       :nemesis (:checker nemesis-package)
                       :workload (:checker workload)})})))

(defn -main [& args]
  (cli/run! (cli/single-test-cmd {:test-fn openraft-test
                                  :opt-fn ensure-random-seed
                                  :opt-spec cli-opts
                                  :usage (cli/test-usage)})
            args))
