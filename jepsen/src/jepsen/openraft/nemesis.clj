(ns jepsen.openraft.nemesis
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
             [generator :as gen]
             [nemesis :as nemesis]
             [random :as random]]
            [jepsen.nemesis.combined :as combined]
            [jepsen.openraft.await :as await]
            [jepsen.openraft.checker :as openraft-checker]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.interruption :as interruption]
            [jepsen.openraft.nemesis.outcome :as outcome]
            [jepsen.openraft.worker :as worker]))

(def ^:private initial-healthy-seconds 5)
(def ^:private retry-interval-seconds 1)
(def ^:private retryable-skip-reasons
  #{:no-process-target
    :no-reachable-pause-target
    :no-safe-packet-target
    :no-safe-partition-target
    :no-supported-leader})

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
          retry-reason (cond
                         (= :skipped (:status completion-value))
                         (:reason completion-value)

                         (and (map? completion-value)
                              (contains? completion-value :status))
                         (:status completion-value)

                         :else
                         completion-value)
          retry? (contains? retryable-skip-reasons retry-reason)
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
    (try
      (let [{:keys [leader]} (cluster/await-ready! test)]
        (info "OpenRaft cluster recovered with leader" leader)
        (assoc op :value (outcome/installed {:leader leader})))
      (catch Exception e
        (cond
          (interruption/interruption? e)
          (do
            (.interrupt (Thread/currentThread))
            (throw e))

          (await/condition-timeout? e :cluster-ready)
          (do
            (info "OpenRaft cluster recovery is indeterminate"
                  {:error (ex-message e)})
            (assoc op
                   :value (outcome/indeterminate
                           :recovery-timeout
                           {:message (ex-message e)})))

          :else
          (throw e)))))

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

(defn- fault-class-checker
  "Reports what one fault class covered in a composed run, without requiring it.

  A composed run draws every fault class from one random schedule, so whether a
  class ever meets a healthy cluster depends on wall-clock timing. The
  single-class jepsen.yml jobs own per-class coverage and keep failing on their
  own missing modes, target roles and changes. A composed run therefore keeps
  the cleanup, error and recovery verdicts and reports the observed and missing
  entries instead of failing on them."
  [fault-checker]
  (openraft-checker/reject-checker-exceptions
   (reify checker/Checker
     (check [_ test history opts]
       (let [result (checker/check fault-checker test history opts)
             observed (or (:observed-modes result)
                          (:observed-target-roles result)
                          (:observed-changes result))
             executed? (boolean (seq observed))
             cluster-state (:cluster-state result)
             valid? (cond
                      (pos? (or (:error-count result) 0)) false
                      (contains? result :restored?)
                      (and (:restored? result)
                           (:recovered? result))
                      (= :intact cluster-state) true
                      (= :unknown cluster-state) :unknown
                      :else false)]
         (assoc result
                :valid? valid?
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
  [failure-state packages]
  (when-not (seq packages)
    (throw (ex-info "At least one nemesis package is required" {})))
  (let [packages (->> packages
                      (mapv (fn [{:keys [name] :as package}]
                              (update (assoc package
                                             :perf
                                             (or (:perf package) #{}))
                                      :nemesis
                                      #(worker/wrap-nemesis-teardown
                                        failure-state
                                        name
                                        %))))
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
                      (openraft-checker/reject-checker-exceptions
                       (checker/compose checkers))))))
