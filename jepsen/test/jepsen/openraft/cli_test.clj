(ns jepsen.openraft.cli-test
  (:require [clojure.test :refer [deftest is testing]]
            [clojure.tools.cli :as tools-cli]
            [jepsen.generator :as gen]
            [jepsen.generator.test :as gen-test]
            [jepsen.openraft.checker :as openraft-checker]
            [jepsen.openraft.cli :as cli]
            [jepsen.openraft.db :as openraft-db]
            [jepsen.openraft.generator :as openraft-generator]
            [jepsen.openraft.harness :as harness]
            [jepsen.openraft.nemesis :as openraft-nemesis]
            [jepsen.openraft.worker :as worker]
            [jepsen.random :as random]))

(deftest records-and-applies-the-random-seed
  (testing "a generated seed is recorded and applied once"
    (let [applied-seeds (atom [])
          parsed (with-redefs [random/set-seed!
                               #(swap! applied-seeds conj %)]
                   (#'cli/ensure-random-seed {:options {}}))]
      (is (integer? (get-in parsed [:options :seed])))
      (is (= [(get-in parsed [:options :seed])]
             @applied-seeds))))

  (testing "test construction does not restart an explicit seed"
    (let [applied-seeds (atom [])
          opts {:nemesis :partition
                :seed 41
                :time-limit 10}]
      (with-redefs [random/set-seed! #(swap! applied-seeds conj %)]
        (let [parsed (#'cli/ensure-random-seed {:options opts})]
          (cli/openraft-test (:options parsed))
          (cli/openraft-test (:options parsed))))
      (is (= [41] @applied-seeds)))))

(deftest validates-the-random-seed
  (testing "a malformed seed is rejected"
    (let [parsed (tools-cli/parse-opts ["--seed" "12e5"] cli/cli-opts)]
      (is (some #(re-find #"Must be an integer" %)
                (:errors parsed)))))

  (testing "the seed remains optional"
    (is (empty? (:errors (tools-cli/parse-opts [] cli/cli-opts))))))

(deftest validates-the-snapshot-threshold
  (testing "the default applies snapshot pressure"
    (is (= openraft-db/default-snapshot-threshold
           (get-in (tools-cli/parse-opts [] cli/cli-opts)
                   [:options :snapshot-threshold]))))

  (testing "malformed and zero thresholds are rejected"
    (doseq [value ["abc" "0"]]
      (let [parsed (tools-cli/parse-opts ["--snapshot-threshold" value]
                                         cli/cli-opts)]
        (is (some #(re-find #"Must be a positive integer" %)
                  (:errors parsed)))))))

(deftest selects-composable-nemeses
  (testing "chaos is the default"
    (is (= [:partition :process :pause :membership]
           (#'cli/normalize-nemeses nil))))

  (testing "comma-separated faults are parsed and canonically ordered"
    (is (= [:partition :process]
           (#'cli/normalize-nemeses
            (#'cli/parse-nemeses "process, partition")))))

  (testing "chaos expands to every composable fault without duplicates"
    (is (= [:partition :process :pause :membership]
           (#'cli/normalize-nemeses [:chaos :partition]))))

  (testing "membership can be combined with another fault"
    (is (#'cli/valid-nemeses? [:membership :partition]))))

(deftest composes-selected-nemesis-checkers
  (let [test (cli/openraft-test {:nemesis [:membership :partition]
                                 :nodes ["n1" "n2" "n3" "n4" "n5"]
                                 :time-limit 10})
        checkers (get-in test [:checker :delegate :checkers])
        nemesis-checkers (get-in checkers [:nemesis :checkers])]
    (is (= #{:seed :stats :exceptions :crash :nemesis :workload}
           (set (keys checkers))))
    (is (= #{:partition :membership}
           (set (keys nemesis-checkers))))))

(deftest shares-one-harness-state-across-workers-and-generators
  (let [failure-state (harness/failure-state)
        wrapped-states (atom [])
        stopped-states (atom [])
        pending-states (atom [])
        composition-states (atom [])
        checker-states (atom [])
        compose-packages openraft-nemesis/compose-packages
        wrap (fn [source]
               (fn [state delegate]
                 (swap! wrapped-states conj [source state])
                 delegate))
        stop (fn [state generator]
               (swap! stopped-states conj state)
               generator)]
    (with-redefs [harness/failure-state (constantly failure-state)
                  openraft-nemesis/compose-packages
                  (fn [state packages]
                    (swap! composition-states conj state)
                    (compose-packages state packages))
                  worker/wrap-db (wrap :db)
                  worker/wrap-client (wrap :client)
                  worker/wrap-nemesis (wrap :nemesis)
                  openraft-checker/reject-harness-failures
                  (fn [state delegate strict-checker-key]
                    (swap! checker-states conj
                           [state strict-checker-key])
                    delegate)
                  openraft-generator/stop-on-harness-failure stop
                  openraft-generator/pending-on-harness-failure
                  (fn [state generator]
                    (swap! pending-states conj state)
                    generator)]
      (cli/openraft-test {:nemesis :partition
                          :nodes ["n1" "n2" "n3"]
                          :time-limit 10}))
    (is (= [:db :client :nemesis] (mapv first @wrapped-states)))
    (is (every? #(identical? failure-state (second %))
                @wrapped-states))
    (is (= 1 (count @stopped-states)))
    (is (identical? failure-state (first @stopped-states)))
    (is (= 1 (count @pending-states)))
    (is (identical? failure-state (first @pending-states)))
    (is (= 1 (count @composition-states)))
    (is (identical? failure-state (first @composition-states)))
    (is (= 1 (count @checker-states)))
    (is (identical? failure-state (ffirst @checker-states)))
    (is (= :exceptions (second (first @checker-states))))))

(defn- lifecycle-test-generator [failure-state]
  (#'cli/lifecycle-generator
   failure-state
   0.3
   {:generator (gen/clients
                (gen/delay 0.1 (repeat {:f :ordinary-workload})))
    :final-generator (gen/clients [{:f :final-workload-1}
                                   {:f :final-workload-2}])}
   {:generator (gen/delay
                 0.1
                 (repeat {:type :info :f :ordinary-nemesis}))
    :final-generator (gen/delay
                       0.3
                       [{:type :info :f :final-nemesis-1}
                        {:type :info :f :final-nemesis-2}])}))

(defn- simulate-lifecycle [failure-state failure-operation]
  (let [operations (atom [])
        failure-recorded? (atom false)
        history (gen-test/simulate
                 (lifecycle-test-generator failure-state)
                 (fn [_context operation]
                   (swap! operations conj (:f operation))
                   (when (and (= failure-operation (:f operation))
                              (compare-and-set! failure-recorded? false true))
                     (harness/record-failure!
                      failure-state
                      :client
                      {:operation operation}
                      (RuntimeException. "client failed")))
                   (-> operation
                       (assoc :type (if (= :nemesis (:process operation))
                                      :info
                                      :ok))
                       (update :time + 10))))]
    {:history history
     :operations @operations}))

(deftest runs-the-full-lifecycle-without-a-harness-failure
  (let [{:keys [history operations]}
        (simulate-lifecycle (harness/failure-state) nil)
        recovery-start (first (filter #(= :final-nemesis-1 (:f %)) history))
        recovery-end (first (filter #(= :final-nemesis-2 (:f %)) history))]
    (is (some #{:ordinary-workload} operations))
    (is (some #{:ordinary-nemesis} operations))
    (is (some (fn [operation]
                (and (= :ordinary-workload (:f operation))
                     (> (:time operation) (:time recovery-start))
                     (< (:time operation) (:time recovery-end))))
              history))
    (is (= [:final-nemesis-1 :final-nemesis-2
            :final-workload-1 :final-workload-2]
           (filter #{:final-nemesis-1 :final-nemesis-2
                     :final-workload-1
                     :final-workload-2}
                   operations)))))

(deftest recovers-and-skips-final-workload-after-a-harness-failure
  (let [failure-state (harness/failure-state)
        {:keys [history operations]}
        (simulate-lifecycle failure-state :ordinary-workload)
        workload-completion-index
        (first (keep-indexed (fn [index operation]
                               (when (and (= :ordinary-workload
                                             (:f operation))
                                          (= :ok (:type operation)))
                                 index))
                             history))
        recovery-index
        (first (keep-indexed (fn [index operation]
                               (when (= :final-nemesis-1 (:f operation))
                                 index))
                             history))]
    (is (some? (harness/primary-failure failure-state)))
    (is (< workload-completion-index recovery-index))
    (is (= 1 (count (filter #{:ordinary-workload} operations))))
    (is (= [:final-nemesis-1 :final-nemesis-2]
           (filter #{:final-nemesis-1 :final-nemesis-2
                     :final-workload-1
                     :final-workload-2}
                   operations)))))

(deftest stops-final-workload-after-a-harness-failure
  (let [failure-state (harness/failure-state)
        {:keys [operations]}
        (simulate-lifecycle failure-state :final-workload-1)]
    (is (some? (harness/primary-failure failure-state)))
    (is (= [:final-nemesis-1 :final-nemesis-2 :final-workload-1]
           (filter #{:final-nemesis-1 :final-nemesis-2
                     :final-workload-1
                     :final-workload-2}
                   operations)))))
