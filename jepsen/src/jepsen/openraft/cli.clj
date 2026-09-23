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
             [clock :as clock]
             [packet :as packet]
             [partition :as partition]
             [process :as process]]
            [jepsen.openraft.scenario [bridge-partition :as bridge-partition]
             [leader-removal-recovery :as leader-removal]
             [two-leaf-partition :as two-leaf-partition]]))

(def ^:private concrete-nemesis-types
  [:partition :process :pause :membership :packet :clock])

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
                     (into concrete-nemesis-types))]
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

(defn- supplied-option? [argv option]
  (some #(or (= option %) (str/starts-with? % (str option "="))) argv))

(def cli-opts
  [[nil "--liveness SCENARIO"
    "Run bridge-partition (10 s window), two-leaf-partition (20 s window), or leader-removal-recovery (60 s recovery deadline). Rejects --nemesis, --packet-mode, and --time-limit."
    :parse-fn keyword
    :validate [#{:bridge-partition :two-leaf-partition :leader-removal-recovery}
               "Must be bridge-partition, two-leaf-partition, or leader-removal-recovery."]]
   [nil "--api-port PORT" "OpenRaft application HTTP port."
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
    "Comma-separated faults: clock, membership, packet, partition, process, pause, or chaos."
    :default [:chaos]
    :parse-fn parse-nemeses
    :validate [valid-nemeses? "Unknown fault."]]

   [nil "--packet-mode MODE" "Packet mode: slow or flaky."
    :parse-fn keyword
    :validate [packet/packet-modes "Must be slow or flaky."]]

   [nil "--seed SEED" "Seed for Jepsen random choices."
    :parse-fn parse-long
    :validate [some? "Must be an integer."]]])

(defn- liveness-node-error [{:keys [liveness nodes]}]
  (when liveness
    (if (= :leader-removal-recovery liveness)
      (when-not (and (<= 2 (count nodes)) (= (count nodes) (count (set nodes))))
        "--liveness=leader-removal-recovery requires at least two distinct nodes.")
      (when-not (= 5 (count nodes) (count (set nodes)))
        "--liveness requires five distinct nodes."))))

(defn- prepare-options [parsed]
  (let [options (:options parsed)
        node-error (liveness-node-error options)
        liveness-errors
        (when (:liveness options)
          (cond-> []
            node-error
            (conj node-error)

            (supplied-option? (:argv options) "--nemesis")
            (conj "--nemesis cannot be used with --liveness.")

            (supplied-option? (:argv options) "--packet-mode")
            (conj "--packet-mode cannot be used with --liveness.")

            (supplied-option? (:argv options) "--time-limit")
            (conj "--time-limit cannot be used with --liveness; each scenario has a fixed observation window.")))
        nemesis-types (when-not (or (seq (:errors parsed)) (:liveness options))
                        (normalize-nemeses (:nemesis options)))
        packet-mode-error
        (when nemesis-types
          (cond
            (and (:packet-mode options)
                 (or (chaos-selection? (:nemesis options))
                     (not (some #{:packet} nemesis-types))))
            "--packet-mode requires an explicit Packet Nemesis."

            (and (= [:packet] nemesis-types)
                 (nil? (:packet-mode options)))
            "--packet-mode is required for Packet Nemesis."

            :else nil))
        seed (or (:seed options)
                 (random/long Long/MAX_VALUE))]
    (random/set-seed! seed)
    (cond-> (assoc-in parsed [:options :seed] seed)
      (seq liveness-errors)
      (update :errors (fnil into []) liveness-errors)

      packet-mode-error
      (update :errors (fnil conj []) packet-mode-error))))

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
  (let [opts (cond-> opts
               (= :leader-removal-recovery (:liveness opts))
               (update :nodes #(vec (take 2 %)))
               (= :two-leaf-partition (:liveness opts))
               (assoc :two-leaf-partition true
                      :bootstrap-state (atom nil)
                      :client-nodes (vec (take 3 (:nodes opts)))))
        failure-state (harness/failure-state)
        database (openraft-db/db opts)
        workload (if (= :leader-removal-recovery (:liveness opts))
                   (select-keys tests/noop-test [:client :checker])
                   (workload/workload opts))
        roles (atom nil)
        nemesis-types (normalize-nemeses (:nemesis opts))
        mode-config
        (cond
          (= :leader-removal-recovery (:liveness opts))
          {:name "openraft leader-removal-recovery"
           :nemesis-package (leader-removal/package database)
           :client (:client workload)
           :generator (leader-removal/generator failure-state)}

          (= :bridge-partition (:liveness opts))
          {:name "openraft bridge-partition"
           :nemesis-package (bridge-partition/package roles)
           :client (:client workload)
           :generator (bridge-partition/generator failure-state workload)}

          (:two-leaf-partition opts)
          {:name "openraft two-leaf-partition"
           :nemesis-package (two-leaf-partition/package roles)
           :client (:client workload)
           :generator (two-leaf-partition/generator failure-state workload)}

          :else
          (let [nemesis-package
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

                           :clock
                           (clock/clock-package)

                           :packet
                           (let [packet-mode (:packet-mode opts)]
                             (when (and (= [:packet] nemesis-types)
                                        (nil? packet-mode))
                               (throw (ex-info
                                       "--packet-mode is required for Packet Nemesis"
                                       {:nemesis nemesis-types})))
                             (packet/packet-package database packet-mode))))
                       nemesis-types))]
            {:name (str "openraft linearizable registers "
                        (str/join "," (map name nemesis-types)))
             :nemesis-package nemesis-package
             :client (:client workload)
             :generator (lifecycle-generator failure-state
                                             (:time-limit opts)
                                             workload
                                             nemesis-package)}))
        nemesis-package (:nemesis-package mode-config)]
    (merge tests/noop-test
           opts
           {:name (:name mode-config)
            :db database
            :client (worker/wrap-client failure-state
                                        (:client mode-config))
            :nemesis (worker/wrap-nemesis failure-state
                                          (:nemesis nemesis-package))
            :generator (:generator mode-config)
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
                                  :opt-fn prepare-options
                                  :opt-spec cli-opts
                                  :usage (cli/test-usage)})
            args))
