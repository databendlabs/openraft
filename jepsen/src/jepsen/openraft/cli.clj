(ns jepsen.openraft.cli
  (:gen-class)
  (:require [clojure.string :as str]
            [jepsen [checker :as checker]
             [cli :as cli]
             [generator :as gen]
             [random :as random]
             [tests :as tests]]
            [jepsen.openraft [checker :as openraft-checker]
             [db :as openraft-db]
             [generator :as openraft-generator]
             [harness :as harness]
             [nemesis :as openraft-nemesis]
             [worker :as worker]
             [workload :as workload]]
            [jepsen.openraft.nemesis [membership :as membership]
             [packet :as packet]
             [partition :as partition]
             [process :as process]]))

(def ^:private concrete-nemesis-types
  [:partition :process :pause :membership :packet])

(def ^:private chaos-nemesis-types
  concrete-nemesis-types)

(def nemesis-types
  (conj (set concrete-nemesis-types) :chaos))

(defn- parse-nemeses [value]
  (mapv (comp keyword str/trim)
        (str/split value #",")))

(defn- normalize-nemeses [selection]
  (let [requested (if (coll? selection)
                    (set selection)
                    #{(or selection :chaos)})
        unknown (remove nemesis-types requested)]
    (when (or (empty? requested) (seq unknown))
      (throw (ex-info "Unknown nemesis selection"
                      {:selection selection
                       :unknown (vec unknown)})))
    (let [expanded (cond-> (disj requested :chaos)
                     (contains? requested :chaos)
                     (into chaos-nemesis-types))]
      (filterv expanded concrete-nemesis-types))))

(defn- valid-nemeses? [selection]
  (try
    (normalize-nemeses selection)
    true
    (catch Exception _
      false)))

(defn- chaos-selection? [selection]
  (or (nil? selection)
      (contains? (set (if (coll? selection)
                        selection
                        [selection]))
                 :chaos)))

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

   [nil "--nemesis TYPES"
    "Comma-separated faults: membership, packet, partition, process, pause, or chaos."
    :default [:chaos]
    :parse-fn parse-nemeses
    :validate [valid-nemeses? "Unknown fault."]]

   [nil "--packet-mode MODE" "Packet mode: slow or flaky."
    :parse-fn keyword
    :validate [packet/packet-modes "Must be slow or flaky."]]

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

(defn- lifecycle-generator
  [failure-state time-limit workload nemesis-package]
  (gen/phases
   (gen/shortest-any
    (gen/nemesis
     (gen/phases
      (openraft-generator/stop-on-harness-failure
       failure-state
       (gen/time-limit time-limit (:generator nemesis-package)))
      (:final-generator nemesis-package)))
    (openraft-generator/pending-on-harness-failure
     failure-state
     (:generator workload)))
   (delay
     (when-not (harness/primary-failure failure-state)
       (openraft-generator/stop-on-harness-failure
        failure-state
        (:final-generator workload))))))

(defn openraft-test [opts]
  (let [failure-state (harness/failure-state)
        database (openraft-db/db opts)
        workload (workload/workload opts)
        chaos? (chaos-selection? (:nemesis opts))
        nemesis-types (normalize-nemeses (:nemesis opts))
        nemesis-package
        (openraft-nemesis/compose-packages
         failure-state
         (mapv (fn [nemesis-type]
                 (case nemesis-type
                   :partition
                   (partition/partition-package)

                   :process
                   (process/process-package database)

                   :pause
                   (process/pause-package database)

                   :membership
                   (membership/membership-package database opts)

                   :packet
                   (let [packet-mode (when-not chaos?
                                       (:packet-mode opts))]
                     (when-not (or chaos? packet-mode)
                       (throw (ex-info
                               "--packet-mode is required for Packet Nemesis"
                               {:nemesis nemesis-types})))
                     (packet/packet-package database packet-mode))))
               nemesis-types))]
    (merge tests/noop-test
           opts
           {:name (str "openraft linearizable registers "
                       (str/join "," (map name nemesis-types)))
            :db database
            :client (worker/wrap-client failure-state (:client workload))
            :nemesis (worker/wrap-nemesis failure-state
                                          (:nemesis nemesis-package))
            :generator (lifecycle-generator failure-state
                                            (:time-limit opts)
                                            workload
                                            nemesis-package)
            :checker (openraft-checker/reject-checker-exceptions
                      (openraft-checker/reject-harness-failures
                       failure-state
                       (openraft-checker/reject-checker-exceptions
                        (checker/compose
                         {:seed (openraft-checker/random-seed-checker)
                          :stats (openraft-checker/reject-checker-exceptions
                                  (checker/stats))
                          :exceptions
                          (openraft-checker/strict-unhandled-exceptions)
                          :crash (openraft-checker/required-log-file-pattern
                                  openraft-checker/node-panic-pattern
                                  "openraft.log")
                          :nemesis (:checker nemesis-package)
                          :workload (:checker workload)}))
                       :exceptions))})))

(defn -main [& args]
  (cli/run! (cli/single-test-cmd {:test-fn openraft-test
                                  :opt-fn ensure-random-seed
                                  :opt-spec cli-opts
                                  :usage (cli/test-usage)})
            args))
