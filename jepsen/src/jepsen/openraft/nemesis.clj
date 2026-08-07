(ns jepsen.openraft.nemesis
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
             [generator :as gen]
             [nemesis :as nemesis]
             [random :as random]]
            [jepsen.nemesis.combined :as combined]
            [jepsen.openraft.cluster :as cluster]))

(def ^:private initial-healthy-seconds 5)
(def ^:private retry-interval-seconds 1)
(def ^:private retryable-skip-results
  #{:no-reachable-pause-target :no-supported-leader})

(defn- jittered-interval [interval]
  (long (random/double (* 0.5 interval)
                       (* 1.5 interval))))

(defrecord IntervalSchedule
           [interval next-time generator retry-generator
            stage operation-f operation-time]
  gen/Generator
  (op [_ test context]
    (when-let [[operation generator'] (gen/op generator test context)]
      (if (= :pending operation)
        [operation (IntervalSchedule. interval
                                      next-time
                                      generator'
                                      retry-generator
                                      stage
                                      operation-f
                                      operation-time)]
        (let [operation (update operation :time max next-time)]
          [operation
           (IntervalSchedule. interval
                              next-time
                              generator'
                              generator
                              :invocation
                              (:f operation)
                              (:time operation))]))))

  (update [_ test context event]
    (let [generator' (gen/update generator test context event)
          retry-generator' (some-> retry-generator
                                   (gen/update test context event))
          completion-value (:value event)
          retry-result (if (map? completion-value)
                         (:status completion-value)
                         completion-value)
          retry? (contains? retryable-skip-results retry-result)
          operation-event? (and stage
                                (= :nemesis (:process event))
                                (= operation-f (:f event)))]
      (cond
        (and operation-event? (= :invocation stage))
        (IntervalSchedule. interval
                           next-time
                           generator'
                           retry-generator'
                           :completion
                           operation-f
                           operation-time)

        (and operation-event? (= :completion stage))
        (IntervalSchedule. interval
                           (+ (if retry? (:time event) operation-time)
                              (jittered-interval
                               (if retry?
                                 (gen/secs->nanos retry-interval-seconds)
                                 interval)))
                           (if retry?
                             retry-generator'
                             generator')
                           nil
                           nil
                           nil
                           nil)

        :else
        (IntervalSchedule. interval
                           next-time
                           generator'
                           retry-generator'
                           stage
                           operation-f
                           operation-time)))))

(defn- interval-schedule
  [interval-seconds initial-delay-seconds generator]
  (IntervalSchedule. (gen/secs->nanos interval-seconds)
                     (gen/secs->nanos initial-delay-seconds)
                     generator
                     nil
                     nil
                     nil
                     nil))

(defrecord RecoveryNemesis []
  nemesis/Nemesis
  (setup! [this _test]
    this)

  (invoke! [_ test op]
    (let [{:keys [leader]} (cluster/await-ready! test)]
      (info "OpenRaft cluster recovered with leader" leader)
      (assoc op :value {:leader leader})))

  (teardown! [this _test]
    this)

  nemesis/Reflection
  (fs [_]
    #{:await-recovery}))

(defn- recovery-package []
  {:nemesis (RecoveryNemesis.)
   :final-generator {:type :info
                     :f :await-recovery}
   :perf #{}})

(defn- fault-class-checker [fault-checker]
  (reify checker/Checker
    (check [_ test history opts]
      (let [result (checker/check fault-checker test history opts)
            observed (or (:observed-modes result)
                         (:observed-changes result))
            executed? (boolean (seq observed))
            cluster-state (:cluster-state result)
            valid? (cond
                     (not executed?) false
                     (pos? (or (:error-count result) 0)) false
                     (contains? result :restored?)
                     (and (:restored? result)
                          (:recovered? result))
                     (= :intact cluster-state) true
                     (= :unknown cluster-state) :unknown
                     :else false)]
        (-> result
            (dissoc :missing-modes :missing-changes)
            (assoc :valid? valid?
                   :fault-class-executed? executed?))))))

(defn- cleanup-order [packages]
  (let [membership? #(= :membership (:name %))]
    (vec (concat (remove membership? packages)
                 (filter membership? packages)))))

(defn- schedule-package [package]
  (let [interval (:interval package)]
    (update package
            :generator
            #(interval-schedule
              interval
              (+ initial-healthy-seconds
                 (random/double interval))
              %))))

(defn compose-packages
  "Composes interval-bearing fault packages and confirms final recovery."
  [packages]
  (when-not (seq packages)
    (throw (ex-info "At least one nemesis package is required" {})))
  (let [packages (->> packages
                      (mapv #(assoc % :perf (or (:perf %) #{})))
                      cleanup-order
                      (mapv schedule-package))
        composed (combined/compose-packages
                  (conj packages (recovery-package)))
        checkers (into {}
                       (map (fn [{:keys [name checker]}]
                              [name (fault-class-checker checker)])
                            packages))]
    (assoc composed
           :nemesis (nemesis/validate (:nemesis composed))
           :checker (if (= 1 (count packages))
                      (:checker (first packages))
                      (checker/compose checkers)))))
