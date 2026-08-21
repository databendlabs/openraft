(ns jepsen.openraft.nemesis.partition
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
             [generator :as gen]
             [nemesis :as nemesis]
             [random :as random]]
            [jepsen.openraft.cluster :as cluster]
            [jepsen.openraft.interruption :as interruption]
            [jepsen.openraft.nemesis.outcome :as outcome]
            [jepsen.openraft.quorum :as quorum]))

(def partition-seconds 10)
(def required-partition-modes
  #{:leader-in-majority :leader-in-minority})

(defn- partition-components [nodes configs leader mode]
  (let [voters (quorum/voter-set configs)]
    (when-not (contains? voters leader)
      (throw (ex-info "The current leader is not a voter"
                      {:leader leader
                       :configs configs})))
    (let [quorums (remove #(= voters %)
                          (quorum/quorum-sets configs))
          candidates (case mode
                       :leader-in-majority
                       (filter #(contains? % leader) quorums)

                       :leader-in-minority
                       (remove #(contains? % leader) quorums)

                       (throw (ex-info "Unknown partition mode"
                                       {:mode mode})))
          quorum-side (first (random/shuffle candidates))]
      (when quorum-side
        (let [other-side (remove quorum-side nodes)]
          (if (contains? quorum-side leader)
            [(vec (filter quorum-side nodes)) (vec other-side)]
            [(vec other-side) (vec (filter quorum-side nodes))]))))))

(defn- invoke-delegate! [delegate test op]
  (try
    (nemesis/invoke! delegate test op)
    (catch Exception e
      ;; Partition control runs multi-node operations on real-pmap workers.
      ;; Restore the interrupt flag on the receiving Nemesis thread too.
      (when (interruption/interruption? e)
        (.interrupt (Thread/currentThread)))
      (throw e))))

(defrecord PartitionNemesis [partitioner]
  nemesis/Nemesis
  (setup! [_ test]
    (PartitionNemesis. (nemesis/setup! partitioner test)))

  (invoke! [_ test op]
    (case (:f op)
      :start-partition
      (if-let [{:keys [leader] :as status}
               (cluster/membership-status test)]
        (let [configs (cluster/voter-configs test status)
              mode (:value op)]
          (if-let [components (partition-components (:nodes test)
                                                    configs
                                                    leader
                                                    mode)]
            (let [grudge (nemesis/complete-grudge components)]
              (info "Partitioning OpenRaft nodes"
                    {:mode mode
                     :leader leader
                     :components components})
              (invoke-delegate! partitioner test
                                (assoc op
                                       :f :start
                                       :value grudge))
              (assoc op
                     :value (outcome/installed
                             {:mode mode
                              :leader leader
                              :voter-configs configs
                              :components components})))
            (do
              (info "Skipping partition without a safe target"
                    {:mode mode
                     :leader leader
                     :voter-configs configs})
              (assoc op
                     :value (outcome/skipped
                             :no-safe-partition-target
                             {:mode mode
                              :leader leader
                              :voter-configs configs})))))
        (do
          (info "Skipping partition without a quorum-supported leader")
          (assoc op
                 :value (outcome/skipped :no-supported-leader {}))))

      :stop-partition
      (do
        (info "Healing OpenRaft network partition")
        (invoke-delegate! partitioner test
                          (assoc op
                                 :f :stop
                                 :value nil))
        (assoc op :value (outcome/installed {})))))

  (teardown! [_ test]
    (try
      (nemesis/teardown! partitioner test)
      (catch Exception e
        (when (interruption/interruption? e)
          (.interrupt (Thread/currentThread)))
        (throw e))))

  nemesis/Reflection
  (fs [_]
    #{:start-partition :stop-partition}))

(defn partition-nemesis []
  (PartitionNemesis. (nemesis/partitioner)))

(defn- legacy-partition-start-installed? [value]
  (and (map? value)
       (required-partition-modes (:mode value))
       (:leader value)
       (coll? (:voter-configs value))
       (coll? (:components value))))

(defn- partition-start-installed? [value]
  (and (outcome/installed-or-legacy? value
                                     legacy-partition-start-installed?)
       (required-partition-modes (:mode value))))

(defn- partition-stop-installed? [value]
  (outcome/installed-or-legacy? value #{:network-healed}))

(defn- recovery-installed? [value]
  (and (outcome/installed-or-legacy?
        value
        #(and (map? %) (:leader %)))
       (:leader value)))

(defn- partition-generator []
  (gen/cycle
   (gen/phases
    {:type :info
     :f :start-partition
     :value :leader-in-majority}
    {:type :info
     :f :stop-partition}
    {:type :info
     :f :start-partition
     :value :leader-in-minority}
    {:type :info
     :f :stop-partition})))

(defn- next-cluster-state
  "Applies one nemesis operation to the partition lifecycle state.

  :intact means the network is healed and the cluster is ready.
  :recovery-pending means a partition exists, or readiness after a heal has not
  been confirmed. :unknown means a partition or heal returned an indeterminate
  result.

  A successful await-recovery does not clear :unknown: readiness only proves
  that every node follows one leader, while leftover rules from an
  indeterminate heal may still cut follower-to-follower links."
  [state op]
  (let [status (get-in op [:value :status])
        operation-error? (boolean (or (:error op)
                                      (:exception op)
                                      (= :indeterminate status)))]
    (case (:f op)
      :start-partition
      (cond
        operation-error? :unknown
        (partition-start-installed? (:value op)) :recovery-pending
        :else state)

      :stop-partition
      (cond
        operation-error? :unknown
        (partition-stop-installed? (:value op)) :recovery-pending
        :else state)

      :await-recovery
      (cond
        operation-error? (if (= :recovery-pending state)
                           :recovery-pending
                           :unknown)
        (= :unknown state) :unknown
        (recovery-installed? (:value op)) :intact
        :else state)

      state)))

(defn- coverage-checker []
  (reify checker/Checker
    (check [_ _test history _opts]
      (let [observed-modes (->> history
                                (filter #(= :start-partition (:f %)))
                                (filter #(partition-start-installed?
                                          (:value %)))
                                (keep #(get-in % [:value :mode]))
                                set)
            missing-modes (remove observed-modes required-partition-modes)
            cluster-state (reduce next-cluster-state :intact history)
            valid? (cond
                     (seq missing-modes) false
                     (= :intact cluster-state) true
                     (= :recovery-pending cluster-state) false
                     :else :unknown)]
        {:valid? valid?
         :observed-modes (vec (sort observed-modes))
         :missing-modes (vec (sort missing-modes))
         :cluster-state cluster-state}))))

(defn partition-package []
  {:name :partition
   :interval partition-seconds
   :nemesis (partition-nemesis)
   :generator (partition-generator)
   :final-generator {:type :info
                     :f :stop-partition}
   :checker (coverage-checker)})
