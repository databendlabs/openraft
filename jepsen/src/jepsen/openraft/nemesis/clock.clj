(ns jepsen.openraft.nemesis.clock
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
             [control :as c]
             [generator :as gen]
             [nemesis :as nemesis]
             [random :as random]]
            [jepsen.openraft.checker :as openraft-checker]
            [jepsen.openraft.clock :as clock]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.nemesis.outcome :as outcome])
  (:import (java.util Locale)))

(def clock-seconds 10)
(def ^:private application-pattern
  "^/usr/local/bin/openraft-jepsen-app([[:space:]].*)?$")
(def ^:private offset-tolerance-ms 1000)
(def ^:private strobe-command "/usr/local/bin/strobe-faketime")

(defn- offset-setting [offset-ms]
  (String/format Locale/ROOT
                 "%+.3fs x1"
                 (to-array [(/ offset-ms 1000.0)])))

(defn- random-offset-ms []
  (long (* (random/nth [-1 1])
           (Math/pow 1.5 (+ 6 (random/double 25))))))

(defn- rate-setting [rate]
  (String/format Locale/ROOT "+0 x%.6f" (to-array [rate])))

(defn- random-rate [direction]
  (Math/pow 2.0
            (case direction
              :fast (random/double 0.0 1.0)
              :slow (random/double -1.0 0.0)
              (throw (ex-info "Unknown clock-rate direction"
                              {:direction direction})))))

(defn- random-strobe-delta-ms []
  (long (Math/pow 2.0 (random/double 2.0 18.0))))

(defn- random-strobe-period-ms []
  (long (Math/pow 2.0 (random/double 6.0 10.0))))

(defn- random-strobe-duration-ms []
  (long (random/double 0.0 32000.0)))

(defn- random-targets [test]
  (vec (random/nonempty-subset (:nodes test))))

(defn verify-offset! [offset-ms]
  (let [before (Long/parseLong (c/exec :date "+%s%3N"))
        observed (clock/probe-wall-time-ms!)
        after (Long/parseLong (c/exec :date "+%s%3N"))
        minimum (- (+ before offset-ms) offset-tolerance-ms)
        maximum (+ after offset-ms offset-tolerance-ms)]
    (when-not (<= minimum observed maximum)
      (throw (ex-info "Clock offset verification failed"
                      {:kind :clock-offset-verification-failed
                       :expected-offset-ms offset-ms
                       :observed-wall-time-ms observed
                       :host-before-ms before
                       :host-after-ms after})))
    {:setting (clock/read-setting!)
     :observed-offset-ms (- observed before)}))

(defn- apply-offsets! [test offsets]
  (c/on-nodes
   test
   (keys offsets)
   (fn [_test node]
     (let [offset-ms (get offsets node)
           setting (offset-setting offset-ms)
           written (clock/write-setting! setting)]
       (when-not (= setting written)
         (throw (ex-info "Clock setting readback failed"
                         {:kind :clock-setting-readback-failed
                          :expected setting
                          :observed written})))
       (assoc (verify-offset! offset-ms)
              :offset-ms offset-ms)))))

(defn- reset-targets! [test targets]
  (c/on-nodes
   test
   targets
   (fn [_test _node]
     (clock/reset-clock!)
     (verify-offset! 0))))

(defn- apply-rates! [test rates]
  (c/on-nodes
   test
   (keys rates)
   (fn [_test node]
     (let [rate (get rates node)
           setting (rate-setting rate)
           written (clock/write-setting! setting)]
       (when-not (= setting written)
         (throw (ex-info "Clock setting readback failed"
                         {:kind :clock-setting-readback-failed
                          :expected setting
                          :observed written})))
       {:setting (clock/read-setting!)
        :rate rate}))))

(defn- strobe-node! [{:keys [delta-ms period-ms duration-ms]}]
  (let [setting (offset-setting delta-ms)
        transitions (max 1 (inc (quot duration-ms period-ms)))
        period-seconds (String/format Locale/ROOT
                                      "%.3f"
                                      (to-array [(/ period-ms 1000.0)]))
        observed (Long/parseLong
                  (c/exec strobe-command
                          clock/control-file
                          setting
                          period-seconds
                          transitions))]
    (when-not (= transitions observed)
      (throw (ex-info "Clock strobe transition count mismatch"
                      {:kind :clock-strobe-transition-count-mismatch
                       :expected transitions
                       :observed observed})))
    (when-not (= clock/normal-setting (clock/read-setting!))
      (throw (ex-info "Clock strobe did not restore normal time"
                      {:kind :clock-strobe-not-restored
                       :observed (clock/read-setting!)})))
    {:delta-ms delta-ms
     :period-ms period-ms
     :duration-ms duration-ms
     :transitions observed
     :setting setting
     :final-setting clock/normal-setting}))

(defn- apply-strobes! [test strobes]
  (try
    (c/on-nodes test
                (keys strobes)
                (fn [_test node]
                  (strobe-node! (get strobes node))))
    (finally
      (reset-targets! test (keys strobes)))))

(defn- leader-evidence [test targets]
  (let [leader (:leader (cluster/membership-status test))]
    {:initial-leader leader
     :leader-included (when leader (boolean (some #{leader} targets)))}))

(defrecord ClockNemesis [settings]
  nemesis/Nemesis
  (setup! [_ test]
    (c/on-nodes test
                (:nodes test)
                (fn [_test _node]
                  (clock/verify-application-clock! application-pattern)))
    (ClockNemesis. (atom (zipmap (:nodes test)
                                 (repeat clock/normal-setting)))))

  (invoke! [_ test op]
    (case (:f op)
      :check-clock
      (assoc op
             :value (outcome/installed
                     {:evidence (reset-targets! test (:nodes test))}))

      :bump-clock
      (let [offsets (:value op)
            targets (vec (keys offsets))
            previous (select-keys @settings targets)
            evidence (apply-offsets! test offsets)
            new-settings (into {} (map (fn [[node offset-ms]]
                                         [node (offset-setting offset-ms)]))
                               offsets)]
        (swap! settings merge new-settings)
        (info "Bumped OpenRaft wall clocks" {:offsets offsets})
        (assoc op
               :value (outcome/installed
                       (merge {:mode :bump
                               :targets targets
                               :target-category (outcome/target-category
                                                 (count (:nodes test))
                                                 (count targets))
                               :offsets-ms offsets
                               :previous-settings previous
                               :settings new-settings
                               :evidence evidence}
                              (leader-evidence test targets)))))

      :rate-clock
      (let [{:keys [direction rates]} (:value op)
            targets (vec (keys rates))
            previous (select-keys @settings targets)
            evidence (apply-rates! test rates)
            new-settings (into {} (map (fn [[node rate]]
                                         [node (rate-setting rate)]))
                               rates)]
        (swap! settings merge new-settings)
        (info "Changed OpenRaft wall-clock rates"
              {:direction direction :rates rates})
        (assoc op
               :value (outcome/installed
                       (merge {:mode :rate
                               :direction direction
                               :targets targets
                               :target-category (outcome/target-category
                                                 (count (:nodes test))
                                                 (count targets))
                               :rates rates
                               :previous-settings previous
                               :settings new-settings
                               :evidence evidence}
                              (leader-evidence test targets)))))

      :strobe-clock
      (let [strobes (:value op)
            targets (vec (keys strobes))
            previous (select-keys @settings targets)
            evidence (apply-strobes! test strobes)]
        (swap! settings merge
               (zipmap targets (repeat clock/normal-setting)))
        (info "Strobed OpenRaft wall clocks" {:strobes strobes})
        (assoc op
               :value (outcome/installed
                       (merge {:mode :strobe
                               :targets targets
                               :target-category (outcome/target-category
                                                 (count (:nodes test))
                                                 (count targets))
                               :strobes strobes
                               :previous-settings previous
                               :setting clock/normal-setting
                               :evidence evidence}
                              (leader-evidence test targets)))))

      :reset-clock
      (let [targets (or (:value op) (:nodes test))
            previous (select-keys @settings targets)
            evidence (reset-targets! test targets)]
        (swap! settings merge (zipmap targets (repeat clock/normal-setting)))
        (assoc op
               :value (outcome/installed
                       {:mode :reset
                        :targets (vec targets)
                        :previous-settings previous
                        :setting clock/normal-setting
                        :evidence evidence})))))

  (teardown! [_ test]
    (reset-targets! test (:nodes test)))

  nemesis/Reflection
  (fs [_]
    #{:check-clock :bump-clock :rate-clock :strobe-clock :reset-clock}))

(defn clock-nemesis []
  (ClockNemesis. (atom {})))

(defn- bump-op [test _context]
  (let [targets (random-targets test)]
    {:type :info
     :f :bump-clock
     :value (zipmap targets (repeatedly random-offset-ms))}))

(defn- reset-op [test _context]
  {:type :info
   :f :reset-clock
   :value (random-targets test)})

(defn- rate-op [direction]
  (fn [test _context]
    (let [targets (random-targets test)]
      {:type :info
       :f :rate-clock
       :value {:direction direction
               :rates (zipmap targets
                              (repeatedly #(random-rate direction)))}})))

(defn- strobe-op [test _context]
  (let [targets (random-targets test)]
    {:type :info
     :f :strobe-clock
     :value (into {}
                  (map (fn [node]
                         [node {:delta-ms (random-strobe-delta-ms)
                                :period-ms (random-strobe-period-ms)
                                :duration-ms (random-strobe-duration-ms)}]))
                  targets)}))

(defn- clock-generator []
  (gen/phases
   {:type :info :f :check-clock}
   (gen/cycle
    (gen/phases
     (gen/once bump-op)
     (gen/once (rate-op :fast))
     (gen/once (rate-op :slow))
     (gen/once strobe-op)
     (gen/once reset-op)))))

(defn- installed? [op]
  (= :installed (get-in op [:value :status])))

(def ^:private required-rate-directions #{:fast :slow})
(def ^:private fault-operations [:bump-clock :rate-clock :strobe-clock])
(def ^:private operation-modes
  {:bump-clock :bump
   :rate-clock :rate
   :strobe-clock :strobe})

(defn- clock-fault-installed?
  "Says whether one clock fault outcome carries the evidence coverage reads.

  A `:rate-clock` outcome counts only when it names the direction it applied,
  because Clock coverage requires both `:fast` and `:slow`. Without that
  check, a rate change with an unusable direction would still contribute the
  `:rate` mode and hide a Nemesis defect."
  [operation-f value]
  (and (= :installed (:status value))
       (= (get operation-modes operation-f) (:mode value))
       (or (not= :rate-clock operation-f)
           (contains? required-rate-directions (:direction value)))))

(defn- coverage-checker []
  (reify checker/Checker
    (check [_ _test history _opts]
      (let [values-by-operation
            (into {}
                  (map (fn [operation-f]
                         [operation-f (->> history
                                           (filter #(= operation-f (:f %)))
                                           (map :value))]))
                  fault-operations)
            recognized-by-operation
            (into {}
                  (map (fn [[operation-f values]]
                         [operation-f
                          (filter #(clock-fault-installed? operation-f %)
                                  values)]))
                  values-by-operation)
            malformed (->> values-by-operation
                           (map (fn [[operation-f values]]
                                  (outcome/malformed-outcomes
                                   #(clock-fault-installed? operation-f %)
                                   values)))
                           (reduce + 0))
            recognized-values (apply concat (vals recognized-by-operation))
            bump? (seq (get recognized-by-operation :bump-clock))
            strobe? (seq (get recognized-by-operation :strobe-clock))
            observed-modes (->> recognized-values
                                (keep :mode)
                                set)
            rate-directions (->> (get recognized-by-operation :rate-clock)
                                 (keep :direction)
                                 set)
            observed-target-categories (->> recognized-values
                                            (keep :target-category)
                                            set)
            final-reset (last (filter #(= :reset-clock (:f %)) history))
            recovered? (and final-reset (installed? final-reset))]
        {:valid? (boolean (and (zero? malformed)
                               bump?
                               strobe?
                               (= required-rate-directions rate-directions)
                               recovered?))
         :bump-installed (boolean bump?)
         :strobe-installed (boolean strobe?)
         :observed-modes observed-modes
         :observed-target-categories
         (vec (sort observed-target-categories))
         :rate-directions (vec (sort rate-directions))
         :malformed-outcomes malformed
         :final-reset-installed (boolean recovered?)
         :cluster-state (if recovered? :intact :unknown)}))))

(defn clock-package []
  {:name :clock
   :interval clock-seconds
   :nemesis (clock-nemesis)
   :generator (clock-generator)
   :final-generator {:type :info
                     :f :reset-clock}
   :checker (openraft-checker/reject-checker-exceptions
             (coverage-checker))})
