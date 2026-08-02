(ns jepsen.openraft.nemesis.partition
  (:require [clojure.tools.logging :refer [info]]
            [jepsen [checker :as checker]
             [generator :as gen]
             [nemesis :as nemesis]
             [random :as random]]
            [jepsen.openraft.cluster :as cluster]
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
      (when-not quorum-side
        (throw (ex-info "Membership has no partition for the requested mode"
                        {:leader leader
                         :mode mode
                         :configs configs})))
      (let [other-side (remove quorum-side nodes)]
        (if (contains? quorum-side leader)
          [(vec (filter quorum-side nodes)) (vec other-side)]
          [(vec other-side) (vec (filter quorum-side nodes))])))))

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
              mode (:value op)
              components (partition-components (:nodes test)
                                               configs
                                               leader
                                               mode)
              grudge (nemesis/complete-grudge components)]
          (info "Partitioning OpenRaft nodes"
                {:mode mode
                 :leader leader
                 :components components})
          (nemesis/invoke! partitioner test
                           (assoc op
                                  :f :start
                                  :value grudge))
          (assoc op
                 :value {:mode mode
                         :leader leader
                         :voter-configs configs
                         :components components}))
        (do
          (info "Skipping partition without a quorum-supported leader")
          (assoc op :value :no-supported-leader)))

      :stop-partition
      (do
        (info "Healing OpenRaft network partition")
        (nemesis/invoke! partitioner test
                         (assoc op
                                :f :stop
                                :value nil))
        (assoc op :value :network-healed))))

  (teardown! [_ test]
    (nemesis/teardown! partitioner test))

  nemesis/Reflection
  (fs [_]
    #{:start-partition :stop-partition}))

(defn partition-nemesis []
  (PartitionNemesis. (nemesis/partitioner)))

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
  (let [operation-error? (boolean (or (:error op)
                                      (:exception op)))]
    (case (:f op)
      :start-partition
      (cond
        operation-error? :unknown
        (get-in op [:value :mode]) :recovery-pending
        :else state)

      :stop-partition
      (cond
        operation-error? :unknown
        (= :network-healed (:value op)) :recovery-pending
        :else state)

      :await-recovery
      (cond
        (= :unknown state) :unknown
        (get-in op [:value :leader]) :intact
        :else state)

      state)))

(defn- coverage-checker []
  (reify checker/Checker
    (check [_ _test history _opts]
      (let [observed-modes (->> history
                                (filter #(= :start-partition (:f %)))
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
